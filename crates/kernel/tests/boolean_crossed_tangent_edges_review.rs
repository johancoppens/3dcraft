//! The review of two tangent edges that cross on a shared cap (random
//! design 121704; docs/design/booleans.md › *Two tangent edges that cross
//! on a shared cap*): the places taken where a blend's tangent edge crosses
//! a border of the other face that lies on one of its faces
//! (`corner::border_crossings`), and hits moved onto them, tried on shapes
//! other than the seed's.
//!
//! The rounded half torus A of the seed (a circle of radius 7.5 about
//! (12.5, 2.5) in z = −10 turned half a turn about the line x = 0,
//! z = −10, its end rim filleted by 2.5) against:
//!
//! - the same moved along the axis by distances that make the two tangent
//!   edges cross steeply, at a sine near the threshold of 1e-2 (the copy
//!   moved by nearly 10, where the two ovals become tangent, or by a tenth
//!   or less), not at all (past 10, within the tolerance and further), and
//!   the distances the builder ran (2.5, 6, 7, 8, 9, 7.4999997);
//! - the same moved across the axis too, so that the edges cross at other
//!   angles;
//! - a copy whose rim is filleted by another radius;
//! - the same moved so that one crossing lies at the original tangent
//!   edge's one vertex, and a few tolerances to either side of it;
//! - a block under the cap's plane whose top edge is filleted (an exact
//!   round, its tangent edge a line on the plane) crossing the tangent edge
//!   steeply, at sines near 1e-2 and below, and within the tolerance of
//!   its top without crossing.
//!
//! Every body built is held to `topo::check`, the boolean's full check and,
//! independently of the kernel, to membership: points sampled about the
//! caps, near the plane, around the crossings and through the whole, each
//! decided in closed form ([`member`]: the half torus less what the fillet
//! removes, the points further than the fillet's radius from the balls'
//! centres, by the distance to their set written out; the block less the
//! corner its fillet removes), against `classify_points` of the body. Where
//! the difference, the intersection and the union are all built, their
//! volumes are held to each other. Refusals are printed, not failures: the
//! review compares them with `main`'s and the candidate's.

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
use craft3d_kernel::boolean::{Place, VerifyOptions, boolean, classify_points, verify};
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_testkit::parallel::map_parallel;
use craft3d_topo::{Body, EdgeId, check, mass_properties};
use std::f64::consts::PI;
use std::time::Instant;

fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Membership written without the kernel, at scale 1, in the operand's own
/// place: `Some(inside)` for a point further than `band` from every face of
/// the solid, `None` within it.
mod member {
    use craft3d_math::Portable;
    use craft3d_math::Vec3;

    const MAJOR: f64 = 12.5;
    const TUBE: f64 = 7.5;
    const CENTRE: f64 = 2.5;

    /// In the plane of the balls' centres (depth `round`), the distance
    /// from `(x, y)` to the region the centres fill there, the points of the
    /// torus of minor radius `spine` at that depth: 0 inside it; outside,
    /// the nearest of 256 points of its border (on the side of `x`), then
    /// Newton on the square of the distance along it.
    fn region(x: f64, y: f64, round: f64, spine: f64) -> f64 {
        let ground = x.phypot(round) - MAJOR;
        if ground * ground + (y - CENTRE) * (y - CENTRE) <= spine * spine {
            return 0.0;
        }
        let side = if x < 0.0 { -1.0 } else { 1.0 };
        let border = |a: f64| {
            let (s, c) = a.psin_cos();
            let rho = MAJOR + spine * c;
            (
                side * (rho * rho - round * round).sqrt(),
                CENTRE + spine * s,
            )
        };
        let mut best = (f64::INFINITY, 0.0);
        for k in 0..256 {
            let a = 2.0 * std::f64::consts::PI * f64::from(k) / 256.0;
            let (bx, by) = border(a);
            let d = (bx - x).phypot(by - y);
            if d < best.0 {
                best = (d, a);
            }
        }
        let mut a = best.1;
        for _ in 0..40 {
            let (s, c) = a.psin_cos();
            let r = MAJOR + spine * c;
            let (dr, ddr) = (-spine * s, -spine * c);
            let b = (r * r - round * round).sqrt();
            let db = r * dr / b;
            let ddb = (dr * dr + r * ddr) / b - (r * dr) * (r * dr) / (b * b * b);
            let (px, dx, ddx) = (side * b - x, side * db, side * ddb);
            let (py, dy, ddy) = (CENTRE + spine * s - y, spine * c, -spine * s);
            let g = px * dx + py * dy;
            let mut h = dx * dx + dy * dy + px * ddx + py * ddy;
            if h <= 0.0 {
                h = dx * dx + dy * dy;
            }
            let step = g / h;
            a -= step;
            if step.abs() < 1e-15 {
                break;
            }
        }
        let (bx, by) = border(a);
        best.0.min((bx - x).phypot(by - y))
    }

    /// The half torus, its end rim (the cap in the half plane x < 0 of
    /// z = −10) filleted by `round`: the half torus less its points with
    /// x < 0 further than `round` from the balls' centres, the points at
    /// depth `round` or more within `7.5 − round` of the tube's centre
    /// circle. Outside that set, the distance to it is the nearer of the
    /// distance to its flat face (in the plane at depth `round`) and, where
    /// the point's own foot on the torus of minor radius `7.5 − round` (in
    /// its plane through the axis) is that deep, the distance to that foot;
    /// otherwise the nearest point is on the edge between the two, which
    /// the flat face's distance covers.
    pub fn rounded(p: Vec3, round: f64, band: f64) -> Option<bool> {
        let depth = -10.0 - p.z;
        let rho = p.x.phypot(p.z + 10.0);
        let t = (rho - MAJOR).phypot(p.y - CENTRE);
        if (t - TUBE).abs() <= band {
            return None;
        }
        if t > TUBE {
            return Some(false);
        }
        if depth.abs() <= band {
            return None;
        }
        if depth < 0.0 {
            return Some(false);
        }
        if p.x >= 0.0 {
            return Some(true);
        }
        let spine = TUBE - round;
        let distance = if depth >= round && t <= spine {
            0.0
        } else {
            let flat = (depth - round).phypot(region(p.x, p.y, round, spine));
            let foot = if t > 0.0 && rho > 0.0 {
                let r = MAJOR + spine * (rho - MAJOR) / t;
                if r * depth / rho >= round {
                    (t - spine).abs()
                } else {
                    f64::INFINITY
                }
            } else {
                f64::INFINITY
            };
            flat.min(foot)
        };
        if (distance - round).abs() <= band {
            return None;
        }
        Some(distance <= round)
    }

    /// The block `x0..x1` by `y0..y1` by `z0..z1`, its edge at `y1`, `z1`
    /// filleted by `round`.
    pub fn block(
        p: Vec3,
        (x0, x1): (f64, f64),
        (y0, y1): (f64, f64),
        (z0, z1): (f64, f64),
        round: f64,
        band: f64,
    ) -> Option<bool> {
        let margins = [p.x - x0, x1 - p.x, p.y - y0, y1 - p.y, p.z - z0, z1 - p.z];
        if margins.iter().any(|m| m.abs() <= band) {
            return None;
        }
        if margins.iter().any(|&m| m < 0.0) {
            return Some(false);
        }
        let (cy, cz) = (y1 - round, z1 - round);
        if p.y > cy && p.z > cz {
            let r = (p.y - cy).phypot(p.z - cz);
            if (r - round).abs() <= band {
                return None;
            }
            return Some(r <= round);
        }
        Some(true)
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

/// The half torus at scale `k`, its end rim filleted by `round` (at scale
/// 1).
fn rounded(k: f64, round: f64) -> Body {
    let tol = tolerance(k);
    let sketch = Frame::WORLD.at(Vec3::new(0.0, 0.0, -10.0 * k)).unwrap();
    let profiles = detect_profiles(&[circle(1, [12.5 * k, 2.5 * k], 7.5 * k)], &tol).unwrap();
    let half = revolve(&profiles, &sketch, [0.0, 0.0], [0.0, 1.0], 0.0, PI, &tol)
        .unwrap()
        .body;
    let rim = edge_near(&half, Vec3::new(-20.0, 2.5, -10.0) * k);
    fillet(
        &half,
        &[FilletEdge {
            edge: rim,
            radius: round * k,
        }],
        &tol,
    )
    .unwrap()
    .body
}

/// The other operand.
#[derive(Clone, Copy, Debug)]
enum Other {
    /// The half torus filleted by `round`, moved by `(dx, dy)` in the
    /// cap's plane.
    Copy { round: f64, dx: f64, dy: f64 },
    /// The block `x0..x1` by `y1 − 9..y1` by `−13..−10`, its top edge at
    /// `y1` filleted by `round`, turned by `turn` about the vertical line
    /// through `pivot`.
    Block {
        x: (f64, f64),
        y1: f64,
        round: f64,
        turn: f64,
        pivot: (f64, f64),
    },
}

impl Other {
    fn build(self, k: f64) -> Body {
        match self {
            Other::Copy { round, dx, dy } => rounded(k, round)
                .transformed(&Transform::translation(Vec3::new(dx, dy, 0.0) * k))
                .unwrap(),
            Other::Block { x, y1, round, .. } => {
                let tol = tolerance(k);
                let frame = Frame::WORLD
                    .at(Vec3::new(x.0, y1 - 9.0, -13.0) * k)
                    .unwrap();
                let block = primitives::block(&frame, (x.1 - x.0) * k, 9.0 * k, 3.0 * k).unwrap();
                let edge = edge_near(&block, Vec3::new(0.5 * (x.0 + x.1), y1, -10.0) * k);
                let made = fillet(
                    &block,
                    &[FilletEdge {
                        edge,
                        radius: round * k,
                    }],
                    &tol,
                )
                .unwrap()
                .body;
                made.transformed(&self.turning(k)).unwrap()
            }
        }
    }

    fn turning(self, k: f64) -> Transform {
        match self {
            Other::Copy { .. } => Transform::translation(Vec3::ZERO),
            Other::Block { turn, pivot, .. } => Transform::rotation_about(
                Vec3::new(pivot.0, pivot.1, -10.0) * k,
                Vec3::new(0.0, 0.0, 1.0),
                turn,
            )
            .unwrap(),
        }
    }

    /// Membership at scale 1.
    fn holds(self, p: Vec3, band: f64) -> Option<bool> {
        match self {
            Other::Copy { round, dx, dy } => {
                member::rounded(p - Vec3::new(dx, dy, 0.0), round, band)
            }
            Other::Block {
                x,
                y1,
                round,
                turn,
                pivot,
            } => {
                let local = Transform::rotation_about(
                    Vec3::new(pivot.0, pivot.1, -10.0),
                    Vec3::new(0.0, 0.0, 1.0),
                    -turn,
                )
                .unwrap()
                .point(p);
                member::block(local, x, (y1 - 9.0, y1), (-13.0, -10.0), round, band)
            }
        }
    }
}

struct Case {
    label: String,
    k: f64,
    turned: bool,
    other: Other,
    ops: Vec<BooleanOp>,
    /// Where the two tangent edges cross in the cap's plane, at scale 1, as
    /// worked out without the kernel (none where unknown).
    crossings: Vec<(f64, f64)>,
    /// vol(A − B) at scale 1 written without the kernel, where known.
    less: Option<f64>,
}

/// The rounded half torus's volume, written without the kernel (the
/// review's own quadrature, `plan/seed-121704/review/tools/indep_volume.py`
/// in the workshop: 6939.565594515954 less what the fillet removes,
/// 55.777737805948; the builder's quadrature and its flux give the same to
/// 1e-12).
const ROUNDED: f64 = 6883.787856710007;

/// vol(A − B) at scale 1 for the copy moved `d` along the axis, written
/// without the kernel by the review's own quadrature (the same tool): in
/// the plane through the axis, π · 12.5 times the lune's area, less the
/// angles A's fillet removes outside B's disc, plus those B's removes and
/// A's does not inside it, each taken from the removed angles as intervals.
/// At 7.5 the builder's quadrature gives 4206.825286588981, 4e-11 apart.
const LESS: [(f64, f64); 6] = [
    (2.5, 1459.716793546542),
    (6.0, 3422.570845471611),
    (7.0, 3950.477813527877),
    (7.5, 4206.825286589024),
    (8.0, 4457.451351160982),
    (9.0, 4939.372840261729),
];

/// Out of the axes: a turn of 0.7 about (1, 2, 3).
fn turn() -> Transform {
    Transform::rotation(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap()
}

/// A seeded generator of numbers in [0, 1).
struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn within(&mut self, (a, b): (f64, f64)) -> f64 {
        a + (b - a) * self.unit()
    }
}

/// The points looked at, at scale 1: about the caps, near the plane,
/// around each crossing and through the whole.
fn samples(case: &Case) -> Vec<Vec3> {
    let (dx, dy) = match case.other {
        Other::Copy { dx, dy, .. } => (dx, dy),
        Other::Block { .. } => (0.0, 0.0),
    };
    let x = (-21.0 + dx.min(0.0), -3.0 + dx.max(0.0));
    let y = (-6.0 + dy.min(0.0), 11.0 + dy.max(0.0));
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut points = Vec::new();
    for _ in 0..500 {
        points.push(Vec3::new(
            rng.within(x),
            rng.within(y),
            rng.within((-13.0, -9.5)),
        ));
    }
    for _ in 0..300 {
        points.push(Vec3::new(
            rng.within(x),
            rng.within(y),
            rng.within((-10.3, -10.0)),
        ));
    }
    for &(cx, cy) in &case.crossings {
        for _ in 0..150 {
            points.push(Vec3::new(
                cx + rng.within((-0.03, 0.03)),
                cy + rng.within((-0.03, 0.03)),
                rng.within((-10.03, -10.0)),
            ));
        }
    }
    for _ in 0..200 {
        points.push(Vec3::new(
            rng.within((-21.0 + dx.min(0.0), 21.0 + dx.max(0.0))),
            rng.within(y),
            rng.within((-31.0, -9.0)),
        ));
    }
    points
}

fn expected(op: BooleanOp, a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match op {
        Subtract => match (a, b) {
            (Some(false), _) | (_, Some(true)) => Some(false),
            (Some(true), Some(false)) => Some(true),
            _ => None,
        },
        Intersect => match (a, b) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Union => match (a, b) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}

/// What a case gave: a line per operation, what is wrong, and the
/// volumes (scale 1) and areas of the operations built.
struct Report {
    lines: Vec<String>,
    wrong: Vec<String>,
    built: Vec<(BooleanOp, bool, f64, f64)>,
}

/// The band about every face within which membership is not decided, at
/// scale 1: a hundred tolerances.
const BAND: f64 = 1e-4;

fn run(case: &Case) -> Report {
    let k = case.k;
    let tol = tolerance(k);
    let place = |p: Vec3| {
        let q = p * k;
        if case.turned { turn().point(q) } else { q }
    };
    let (mut a, mut b) = (rounded(k, 2.5), case.other.build(k));
    if case.turned {
        a = a.transformed(&turn()).unwrap();
        b = b.transformed(&turn()).unwrap();
    }
    let measure = |body: &Body| {
        let mass = mass_properties(body).unwrap();
        (mass.volume / (k * k * k), mass.area / (k * k))
    };
    let (whole_a, whole_b) = (measure(&a), measure(&b));
    let points = samples(case);
    let placed: Vec<Vec3> = points.iter().map(|&p| place(p)).collect();
    let mut report = Report {
        lines: Vec::new(),
        wrong: Vec::new(),
        built: Vec::new(),
    };
    for &op in &case.ops {
        // The difference both ways: A − B and B − A.
        for swapped in [false, true] {
            if swapped && op != Subtract {
                continue;
            }
            let (first, second) = if swapped { (&b, &a) } else { (&a, &b) };
            let name = match (op, swapped) {
                (Subtract, false) => "A-B",
                (Subtract, true) => "B-A",
                (Intersect, _) => "A*B",
                (Union, _) => "A+B",
            };
            let label = format!("{} {name}", case.label);
            let start = Instant::now();
            let made = boolean(first, second, op, &tol);
            let seconds = start.elapsed().as_secs_f64();
            let body = match made {
                Ok(made) => made.body,
                Err(e) => {
                    report
                        .lines
                        .push(format!("{label}: refused ({seconds:.2} s): {e}"));
                    continue;
                }
            };
            let mut problems: Vec<String> =
                check(&body, &tol).iter().map(ToString::to_string).collect();
            match verify(first, second, op, &body, &tol, &VerifyOptions::default()) {
                Ok(found) => problems.extend(found.violations.iter().map(ToString::to_string)),
                Err(e) => problems.push(format!("the boolean's check cannot run: {e}")),
            }
            let (mut decided, mut unknown, mut mismatched) = (0, 0, Vec::new());
            match classify_points(&body, &placed, &tol) {
                Ok(places) => {
                    for (p, place) in points.iter().zip(places) {
                        let (pa, pb) = (member::rounded(*p, 2.5, BAND), case.other.holds(*p, BAND));
                        let (pa, pb) = if swapped { (pb, pa) } else { (pa, pb) };
                        let Some(want) = expected(op, pa, pb) else {
                            continue;
                        };
                        let got = match place {
                            Place::Inside => true,
                            Place::Outside => false,
                            Place::OnBoundary | Place::Unknown => {
                                unknown += 1;
                                continue;
                            }
                        };
                        decided += 1;
                        if got != want {
                            mismatched.push(*p);
                        }
                    }
                }
                Err(e) => problems.push(format!("points cannot be classified: {e}")),
            }
            if !mismatched.is_empty() {
                problems.push(format!(
                    "{} of {decided} points placed against membership, first {:?}",
                    mismatched.len(),
                    &mismatched[..mismatched.len().min(4)]
                ));
            }
            let mass = mass_properties(&body).unwrap();
            let (volume, area) = (mass.volume / (k * k * k), mass.area / (k * k));
            // The copy moved along the axis: B − A is A − B's mirror image
            // (in y = 2.5 + d / 2), A ∩ B is A less A − B, A ∪ B is A and
            // A − B.
            if let Some(less) = case.less {
                let want = match op {
                    Subtract => less,
                    Intersect => ROUNDED - less,
                    Union => ROUNDED + less,
                };
                if (volume - want).abs() > 1e-6 * area {
                    problems.push(format!(
                        "volume {volume} where {want} is written without the kernel"
                    ));
                }
            }
            report.lines.push(format!(
                "{label}: built ({seconds:.2} s), V-E-F {}-{}-{}, {} shell(s), volume {volume:.9}, area {area:.6}, points {decided} decided, {unknown} on the boundary, {} wrong{}",
                body.vertices.len(),
                body.edges.len(),
                body.faces.len(),
                body.shells.len(),
                mismatched.len(),
                if problems.is_empty() {
                    String::new()
                } else {
                    format!(": {problems:?}")
                }
            ));
            if !problems.is_empty() {
                report
                    .wrong
                    .push(format!("{label}: built, but {problems:?}"));
            }
            report.built.push((op, swapped, volume, area));
        }
    }
    // The volumes of the operations held to each other where all are built.
    let find = |op: BooleanOp, swapped: bool| {
        report
            .built
            .iter()
            .find(|&&(o, s, _, _)| o == op && s == swapped)
            .map(|&(_, _, v, a)| (v, a))
    };
    // vol A = vol(A − B) + vol(A ∩ B), and vol B = vol(B − A) + vol(A ∩ B).
    for (swapped, (whole, whole_area)) in [(false, whole_a), (true, whole_b)] {
        if let (Some((less, la)), Some((common, ca))) =
            (find(Subtract, swapped), find(Intersect, false))
        {
            let gap = (whole - less - common).abs();
            let allowed = 1e-6 * (whole_area + la + ca);
            let line = format!(
                "{}: {} {whole:.9} against its two parts {:.9}, {gap:.1e} apart, allowed {allowed:.1e}",
                case.label,
                if swapped { "B" } else { "A" },
                less + common
            );
            if gap > allowed {
                report.wrong.push(line.clone());
            }
            report.lines.push(line);
        }
    }
    if let (Some((less, la)), Some((common, ca)), Some((union, ua)), Some((other, oa))) = (
        find(Subtract, false),
        find(Intersect, false),
        find(Union, false),
        find(Subtract, true),
    ) {
        // vol A = vol(A − B) + vol(A ∩ B); vol B = vol(B − A) + vol(A ∩ B);
        // vol(A ∪ B) = vol(A − B) + vol(B − A) + vol(A ∩ B).
        let gap = (union - less - other - common).abs();
        let allowed = 1e-6 * (la + ca + ua + oa);
        let line = format!(
            "{}: union {union:.9} against the three parts {:.9}, {gap:.1e} apart, allowed {allowed:.1e}",
            case.label,
            less + other + common
        );
        if gap > allowed {
            report.wrong.push(line.clone());
        }
        report.lines.push(line);
    }
    report
}

fn survey(cases: Vec<Case>) {
    let reports = map_parallel(cases, |case| run(&case));
    let mut wrong = Vec::new();
    for report in reports {
        for line in &report.lines {
            println!("{line}");
        }
        wrong.extend(report.wrong);
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where the tangent edges of the half torus and of its copy moved `d`
/// along the axis cross in the cap's plane: on y = 2.5 + d / 2, where the
/// ovals the balls' centres stand over meet.
fn crossings_along(d: f64) -> Vec<(f64, f64)> {
    let h = 25.0 - 0.25 * d * d;
    if h < 0.0 {
        return Vec::new();
    }
    [12.5 + h.sqrt(), 12.5 - h.sqrt()]
        .into_iter()
        .map(|r| (-(r * r - 6.25).sqrt(), 2.5 + 0.5 * d))
        .collect()
}

/// Where the half torus's tangent edge crosses the line y = `c` in the
/// cap's plane.
fn crossings_on_line(c: f64) -> Vec<(f64, f64)> {
    let h = 25.0 - (c - 2.5) * (c - 2.5);
    if h < 0.0 {
        return Vec::new();
    }
    [12.5 + h.sqrt(), 12.5 - h.sqrt()]
        .into_iter()
        .map(|r| (-(r * r - 6.25).sqrt(), c))
        .collect()
}

fn along(d: f64, k: f64, turned: bool, ops: &[BooleanOp]) -> Case {
    Case {
        label: format!("moved {d} k={k}{}", if turned { " turned" } else { "" }),
        k,
        turned,
        other: Other::Copy {
            round: 2.5,
            dx: 0.0,
            dy: d,
        },
        ops: ops.to_vec(),
        crossings: crossings_along(d),
        less: LESS
            .iter()
            .find(|&&(at, _)| (at - d).abs() < 1e-12)
            .map(|&(_, less)| less),
    }
}

/// The copy moved along the axis: steeply crossing, near the sine of
/// 1e-2 either way (the ovals tangent at 10 apart: about 2 √((10 − d) / 5),
/// 1.3e-2 at 9.9998, 8.9e-3 at 9.9999), the builder's distances, and past
/// 10 not crossing at all.
#[test]
fn moved_along_the_axis() {
    let all = [Subtract, Intersect, Union];
    let mut cases = Vec::new();
    for d in [
        2.5, 5.0, 6.0, 7.0, 7.4999997, 7.5, 7.5000003, 8.0, 9.0, 9.9, 9.99, 9.999, 9.9995, 9.9998,
        9.9999, 10.0, 10.0000005, 10.00001, 10.5,
    ] {
        cases.push(along(d, 1.0, false, &all));
    }
    survey(cases);
}

/// The copy moved a little along the axis: the tangent edges cross near
/// the ovals' far sides at a sine of about d / 5 (2e-2 at 0.1, 8e-3 at
/// 0.04), the tori nearly one.
#[test]
fn moved_a_little_along_the_axis() {
    let mut cases = Vec::new();
    for d in [0.04, 0.1, 0.2] {
        cases.push(along(d, 1.0, false, &[Subtract, Intersect]));
    }
    survey(cases);
}

/// The same at other scales and turned out of the axes.
#[test]
fn moved_along_the_axis_at_other_scales() {
    let mut cases = Vec::new();
    for (d, k) in [
        (7.0, 1e3),
        (9.0, 1e-3),
        (9.9998, 1e3),
        (9.9999, 1e-3),
        (8.0, 1e6),
    ] {
        cases.push(along(d, k, true, &[Subtract, Intersect]));
    }
    survey(cases);
}

/// The copy moved across the axis too: the tangent edges cross at other
/// angles, and the tori's axes are apart.
#[test]
fn moved_across_the_axis() {
    let mut cases = Vec::new();
    for (dx, dy) in [
        (1.0, 7.5),
        (-2.0, 6.0),
        (3.0, 3.0),
        (0.7, 9.0),
        (-4.0, 1.5),
        (2.0, 9.95),
    ] {
        cases.push(Case {
            label: format!("moved ({dx}, {dy})"),
            k: 1.0,
            turned: false,
            other: Other::Copy { round: 2.5, dx, dy },
            ops: vec![Subtract, Intersect],
            crossings: Vec::new(),
            less: None,
        });
    }
    survey(cases);
}

/// A copy whose rim is filleted by another radius.
#[test]
fn a_copy_filleted_by_another_radius() {
    let mut cases = Vec::new();
    for (round, d) in [(2.0, 7.5), (1.0, 7.5), (3.0, 6.0), (2.0, 9.0)] {
        cases.push(Case {
            label: format!("copy rounded {round} moved {d}"),
            k: 1.0,
            turned: false,
            other: Other::Copy {
                round,
                dx: 0.0,
                dy: d,
            },
            ops: vec![Subtract, Intersect],
            crossings: Vec::new(),
            less: None,
        });
    }
    survey(cases);
}

/// The copy moved so that the outer crossing lies at the original tangent
/// edge's one vertex, and `m` tolerances along the edge to either side of
/// it (the place is kept only clear of the edge's ends by twice the
/// tolerance).
#[test]
fn a_crossing_at_the_tangent_edges_vertex() {
    let a = rounded(1.0, 2.5);
    let near = Vec3::new(-17.256, 3.294, -10.0);
    let vertex = a
        .vertices
        .iter()
        .map(|v| v.point)
        .min_by(|p, q| p.distance(near).total_cmp(&q.distance(near)))
        .unwrap();
    println!("the tangent edge's vertex: {vertex:?}");
    let at = 2.0 * (vertex.y - 2.5);
    let mut cases = Vec::new();
    for m in [-3.0, -1.0, 0.0, 0.5, 1.0, 2.0, 3.0, 5.0, 20.0] {
        // The crossing moves along y by half the move, and the edge there
        // runs nearly along y.
        let d = at + 2.0 * m * 1e-6;
        let mut case = along(d, 1.0, false, &[Subtract, Intersect]);
        case.label = format!("crossing {m} tolerances from the vertex (moved {d})");
        cases.push(case);
    }
    survey(cases);
}

/// A block under the cap's plane, its top face on it and its top edge at
/// `y1` filleted by 1 (an exact round whose tangent edge is the line
/// y = y1 − 1 on the plane): the half torus's tangent edge crosses that
/// line steeply, at sines of about 6e-2, 1.4e-2 and 6e-3 near the top of
/// its oval (y = 7.5), and passes within the tolerance of it without
/// crossing.
#[test]
fn a_block_whose_top_edge_is_filleted() {
    let mut cases = Vec::new();
    for (c, turn) in [
        (4.0, 0.0),
        (4.0, 0.3),
        (7.0, 0.0),
        (7.49, 0.0),
        (7.4995, 0.0),
        (7.4999, 0.0),
        (7.5 - 3e-7, 0.0),
        (7.5 + 3e-7, 0.0),
    ] {
        cases.push(Case {
            label: format!("block rounded to y={c} turned {turn}"),
            k: 1.0,
            turned: false,
            other: Other::Block {
                x: (-22.0, -2.0),
                y1: c + 1.0,
                round: 1.0,
                turn,
                pivot: (-12.25, c),
            },
            ops: vec![Subtract, Intersect, Union],
            crossings: if turn.abs() < 1e-12 {
                crossings_on_line(c)
            } else {
                Vec::new()
            },
            less: None,
        });
    }
    survey(cases);
}
