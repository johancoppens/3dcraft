//! A ball far larger than the part it is sunk into (2026-11-21, found by
//! the review of the shallow crossing's repair: docs/design/booleans.md ›
//! *A surface far larger than the box*).
//!
//! A ball of radius 1e3 to 1e6 with its surface a few tolerances inside
//! the side of a cylinder of radius 4 or of a cone 4 by 12 crosses that
//! side in a thin loop: hundreds of times longer than wide, turning at
//! its two tips within a few tens of tolerances, where the surfaces cross
//! at a sine of a few 1e-5. The pair is asked as a boolean asks it: in
//! the part of the side's box the ball's box holds, grown by a share of
//! the larger face's box (thousands, for the ball) and by a share of the
//! smaller.
//!
//! What was answered, before: where the box is so large that rounding at
//! its size is the nearest a point is brought to the other surface, the
//! march's points lay anywhere across a band wider than the tip's steps,
//! it passed its own start unseen, went round the loop a second time and
//! closed there: one closed curve twice round, which a boolean reads as
//! nothing in common (the pairs answered *clear*). In the smaller box a
//! point was held nearer the ball than the ball's distance, the
//! difference of two numbers of 1e5, can be told: the march stalled.
//!
//! The truth here is the placement itself. The ball's distance is taken
//! exactly (the squares by fused multiplication, summed with their
//! errors), the loop is found along rays from its middle by halving, and
//! every point of a returned curve is held to the crossing along the
//! side: not only to both surfaces, which lie within the tolerance of
//! each other all over a loop 5 tolerances deep.

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
use craft3d_geom::intersect::{MeetingCurve, SurfaceMeeting, meet_surfaces};
use craft3d_math::{Bbox3, Frame, Portable, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

const TOL: f64 = 1e-6;
/// How far along the side a point of a curve may be from the crossing, in
/// tolerances: one (0.503 at most over the sweep, 2026-11-21; before, 63
/// of its questions came back with a curve 2 to 12.6 tolerances from the
/// crossing, on both surfaces).
const ALONG: f64 = 1.0;
/// The part's radius: the cylinder's, and the cone's at its base.
const R_PART: f64 = 4.0;
/// The cone's height.
const HEIGHT: f64 = 12.0;

/// A ball of `radius` with its surface `sunk` tolerances inside the side
/// of the cone or of the cylinder (outside it where negative).
#[derive(Clone, Copy, Debug)]
struct Case {
    cone: bool,
    radius: f64,
    sunk: f64,
}

impl Case {
    fn depth(&self) -> f64 {
        self.sunk * TOL
    }

    /// The cone's slant, and its side's outward normal in `(ρ, z)`.
    fn slope() -> (f64, (f64, f64)) {
        let slant = HEIGHT.phypot(R_PART);
        (slant, (HEIGHT / slant, R_PART / slant))
    }

    /// The part's side: the cylinder of radius 4 about z (its body from
    /// z = −5 to 5), or the cone of radius 4 at z = 0 with its apex at
    /// z = 12; and the box of its face.
    fn side(&self) -> (Surface, Bbox3) {
        if self.cone {
            let half_angle = -(R_PART / HEIGHT).patan();
            (
                cone(Frame::WORLD, R_PART, half_angle),
                Bbox3::from_points([
                    Vec3::new(-R_PART, -R_PART, 0.0),
                    Vec3::new(R_PART, R_PART, HEIGHT),
                ]),
            )
        } else {
            let frame = Frame::WORLD.at(Vec3::new(0.0, 0.0, -5.0)).unwrap();
            (
                cylinder(frame, R_PART),
                Bbox3::from_points([
                    Vec3::new(-R_PART, -R_PART, -5.0),
                    Vec3::new(R_PART, R_PART, 5.0),
                ]),
            )
        }
    }

    /// The place on the side the ball is sunk in at: `(ρ, z)`.
    fn foot(&self) -> (f64, f64) {
        if self.cone { (2.0, 6.0) } else { (R_PART, 0.7) }
    }

    /// The ball's centre: along the side's normal at the foot, the radius
    /// less the depth from it.
    fn centre(&self) -> Vec3 {
        let (rho, z) = self.foot();
        let reach = self.radius - self.depth();
        if self.cone {
            let (_, n) = Self::slope();
            Vec3::new(rho + n.0 * reach, 0.0, z + n.1 * reach)
        } else {
            Vec3::new(rho + reach, 0.0, z)
        }
    }

    fn ball(&self) -> Surface {
        sphere(Frame::WORLD.at(self.centre()).unwrap(), self.radius)
    }

    /// The boxes a boolean asks the pair in: the part of the side's box
    /// the ball's box holds, grown by the room of the larger face (the
    /// ball's), and by the room of the smaller.
    fn boxes(&self) -> [Bbox3; 2] {
        let (_, face) = self.side();
        let (lo, hi) = face.corners().unwrap();
        let c = self.centre();
        let r = self.radius;
        let lo = Vec3::new(lo.x.max(c.x - r), lo.y.max(c.y - r), lo.z.max(c.z - r));
        let hi = Vec3::new(hi.x.min(c.x + r), hi.y.min(c.y + r), hi.z.min(c.z + r));
        [2.0 * r * 3.0_f64.sqrt(), face.diagonal()].map(|diagonal| {
            let room = 16.0 * TOL + 0.013 * diagonal;
            let pad = Vec3::new(room, room, room);
            Bbox3::from_points([lo - pad, hi + pad])
        })
    }

    /// The side's point at the angle `u` about its axis and the height
    /// `z`, with its radius there.
    fn at(&self, u: f64, z: f64) -> Vec3 {
        let rho = if self.cone {
            R_PART * (1.0 - z / HEIGHT)
        } else {
            R_PART
        };
        let (s, c) = u.psin_cos();
        Vec3::new(rho * c, rho * s, z)
    }

    /// The side's signed distance at `q` (outside positive) and its
    /// outward normal there.
    fn side_at(&self, q: Vec3) -> (f64, Vec3) {
        let rho = q.x.phypot(q.y);
        let radial = Vec3::new(q.x / rho, q.y / rho, 0.0);
        if self.cone {
            let (_, n) = Self::slope();
            (
                (rho - R_PART * (1.0 - q.z / HEIGHT)) * n.0,
                radial * n.0 + Vec3::Z * n.1,
            )
        } else {
            (rho - R_PART, radial)
        }
    }

    /// The ball's signed distance at `q`, exactly, and its outward normal.
    fn ball_at(&self, q: Vec3) -> (f64, Vec3) {
        let c = self.centre();
        (
            ball_distance(q, c, self.radius),
            (q - c).normalized().unwrap(),
        )
    }

    /// How far `q`, a point near both surfaces, is from where they cross:
    /// to the side by the side's distance, and along it by the gap
    /// between the surfaces over the sine of the angle they cross at.
    fn off_the_crossing(&self, q: Vec3) -> f64 {
        let (side, n) = self.side_at(q);
        let (ball, m) = self.ball_at(q);
        let (sine, cosine) = (n.cross(m).length(), n.dot(m));
        side.phypot((ball - side * cosine) / sine)
    }

    /// `so_far`, or how far `q` is from the crossing and from the further
    /// of the two surfaces where that is more.
    fn furthest(&self, q: Vec3, so_far: (f64, f64)) -> (f64, f64) {
        let off = self.side_at(q).0.abs().max(self.ball_at(q).0.abs());
        (so_far.0.max(self.off_the_crossing(q)), so_far.1.max(off))
    }

    /// The half sizes of the loop in the side's `(u, z)`: around the axis
    /// and along it. (Sizes to scale by, not held to.)
    fn half_sizes(&self) -> (f64, f64) {
        let d = self.depth().max(0.5 * TOL);
        let (rho, _) = self.foot();
        let (along, across) = if self.cone {
            let (_, n) = Self::slope();
            // Along a ruling, which rises `n.0` in z per unit length; the
            // side's radius of curvature across it is `ρ / n.0`.
            (n.0, rho / n.0)
        } else {
            (1.0, rho)
        };
        let across = (2.0 * d * across * self.radius / (across + self.radius)).sqrt();
        (across / rho, (2.0 * d * self.radius).sqrt() * along)
    }

    /// The angle about the loop's middle of a point of the side, in the
    /// side's `(u, z)` scaled by the loop's half sizes.
    fn angle_of(&self, q: Vec3) -> f64 {
        let (ku, kz) = self.half_sizes();
        let (_, z) = self.foot();
        ((q.z - z) / kz).patan2(q.y.patan2(q.x) / ku)
    }

    /// The point of the loop in the direction `phi` from its middle (in
    /// the scaled `(u, z)`), by halving on the ball's exact distance
    /// along the ray; `None` where the ray leaves the box, or the side,
    /// before the loop.
    fn loop_point(&self, phi: f64, within: &Bbox3) -> Option<Vec3> {
        let (ku, kz) = self.half_sizes();
        let (_, z) = self.foot();
        let (s, c) = phi.psin_cos();
        let at = |t: f64| self.at(ku * t * c, z + kz * t * s);
        let inside = |t: f64| {
            let q = at(t);
            (!self.cone || q.z < HEIGHT - 1e-3) && within.contains(q, 0.0)
        };
        // As far as the ray stays in the box, no further than 8 half
        // sizes: the loop is about 1 from the middle.
        let (mut lo, mut hi) = (0.0, 8.0);
        if !inside(hi) {
            let (mut a, mut b) = (0.0, hi);
            for _ in 0..80 {
                let mid = 0.5 * (a + b);
                if inside(mid) {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            hi = a;
        }
        if !(self.ball_at(at(lo)).0 < 0.0 && self.ball_at(at(hi)).0 > 0.0) {
            return None;
        }
        for _ in 0..64 {
            let mid = 0.5 * (lo + hi);
            if self.ball_at(at(mid)).0 < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(at(0.5 * (lo + hi)))
    }
}

/// `a + b` with what rounding took from it.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// `|q − c| − r`, exactly to the last bits of `q`: `(|q − c|² − r²) /
/// (|q − c| + r)`, every square with its rounding error by a fused
/// multiplication, the terms summed from the largest down with the error
/// of every addition kept.
fn ball_distance(q: Vec3, c: Vec3, r: f64) -> f64 {
    let mut terms: Vec<f64> = Vec::new();
    let mut square = |a: f64, sign: f64| {
        let p = a * a;
        terms.push(sign * p);
        terms.push(sign * a.pmul_add(a, -p));
    };
    let mut small = Vec::new();
    for (qk, ck) in [(q.x, c.x), (q.y, c.y), (q.z, c.z)] {
        let (h, l) = two_sum(qk, -ck);
        square(h, 1.0);
        small.push(2.0 * h * l + l * l);
    }
    square(r, -1.0);
    terms.extend(small);
    terms.sort_by(|a, b| b.abs().total_cmp(&a.abs()));
    let (mut sum, mut lost) = (0.0, 0.0);
    for term in terms {
        let (s, e) = two_sum(sum, term);
        sum = s;
        lost += e;
    }
    (sum + lost) / (q.distance(c) + r)
}

/// The angles of a curve about the loop's middle, sampled until no two
/// neighbours are more than a twentieth of a degree apart (the curve is
/// parametrised by its length, and at a tip of the loop the angle turns
/// by a right angle within a few tens of tolerances), with the furthest
/// any sample is from the crossing and from either surface.
fn walk(case: &Case, curve: &MeetingCurve) -> (Vec<f64>, f64, f64) {
    fn between(
        case: &Case,
        curve: &MeetingCurve,
        (t0, a0): (f64, f64),
        (t1, a1): (f64, f64),
        depth: u32,
        out: &mut Vec<f64>,
        worst: &mut (f64, f64),
    ) {
        let turn = (a1 - a0 + PI).rem_euclid(TAU) - PI;
        if turn.abs() <= 0.05_f64.to_radians() || depth >= 40 {
            return;
        }
        let t = 0.5 * (t0 + t1);
        let q = curve.curve.eval(t);
        let a = case.angle_of(q);
        *worst = case.furthest(q, *worst);
        between(case, curve, (t0, a0), (t, a), depth + 1, out, worst);
        out.push(a);
        between(case, curve, (t, a), (t1, a1), depth + 1, out, worst);
    }
    let (t0, t1) = curve.range;
    let mut out = Vec::new();
    let mut worst = (0.0_f64, 0.0_f64);
    let n = 256;
    let mut before: Option<(f64, f64)> = None;
    for k in 0..=n {
        let t = t0 + (t1 - t0) * f64::from(k) / f64::from(n);
        let q = curve.curve.eval(t);
        let a = case.angle_of(q);
        worst = case.furthest(q, worst);
        if let Some(b) = before {
            between(case, curve, b, (t, a), 0, &mut out, &mut worst);
        }
        out.push(a);
        before = Some((t, a));
    }
    (out, worst.0, worst.1)
}

/// Whether the answer in `within` is right (`Ok`, with the furthest a
/// point of a curve is from the crossing, in tolerances), refused (`Err`
/// that begins with "refused") or wrong.
fn verdict(case: &Case, within: &Bbox3) -> Result<f64, String> {
    let (side, _) = case.side();
    let tol = Tolerance::new(TOL, 1e-9).unwrap();
    let met = match meet_surfaces(&side, &case.ball(), within, &tol) {
        Ok(met) => met,
        Err(e) => return Err(format!("refused: {e}")),
    };
    // Outside by more than the tolerance: nothing.
    if case.sunk <= -1.5 {
        return match met {
            SurfaceMeeting::None => Ok(0.0),
            other => Err(format!(
                "WRONG: apart, and met: {}",
                format!("{other:?}").chars().take(120).collect::<String>()
            )),
        };
    }
    let m = match met {
        SurfaceMeeting::Meets(m) => m,
        other => return Err(format!("WRONG: {} tolerances in, and {other:?}", case.sunk)),
    };
    // Within the tolerance of each other and no deeper: a touch. A point
    // on both; a loop only where the ball is inside, and then the loop.
    for p in &m.points {
        let (on_side, on_ball) = (case.side_at(*p).0.abs(), case.ball_at(*p).0.abs());
        if !(on_side <= TOL && on_ball <= TOL) {
            return Err(format!(
                "WRONG: a point {on_side:e} from the side and {on_ball:e} from the ball"
            ));
        }
    }
    let touch = case.sunk.abs() <= 0.5;
    // Between half and twice the tolerance inside, a boolean takes the
    // pair for neither touching nor safely crossing and refuses it: there
    // the loop may come with points where the surfaces touch (a ball of
    // 1e6 sunk 1.5 tolerances crosses the cone's side at a sine of
    // 1.7e-6 at the loop's tips, below what the marcher reads as a
    // crossing, and comes back as the loop and its two tips). Curves are
    // held to the crossing and points to both surfaces all the same.
    let band = case.sunk > 0.5 && case.sunk < 2.0;
    if (touch || band) && m.curves.is_empty() {
        return if m.points.is_empty() {
            Err("WRONG: within the tolerance, and nothing met".to_string())
        } else {
            Ok(0.0)
        };
    }
    if case.sunk < 0.0 || (!touch && !band && !m.points.is_empty()) {
        return Err(format!(
            "WRONG: {} curves and {} points",
            m.curves.len(),
            m.points.len()
        ));
    }
    // The loop: every point of every curve on both surfaces and on the
    // crossing; every direction from the loop's middle in which the loop
    // is inside the box passed by a curve; and passed once.
    if m.curves.is_empty() {
        return Err("WRONG: no curve".to_string());
    }
    const BINS: usize = 720;
    let mut passed = vec![0.0_f64; BINS];
    let mut worst = 0.0_f64;
    for curve in &m.curves {
        let (angles, off, off_a_surface) = walk(case, curve);
        worst = worst.max(off);
        // On both surfaces within what the curve says, which is within
        // the tolerance; its ends in the box; closed if it says so.
        let (first, last) = (start(curve), end(curve));
        if !(curve.deviation <= TOL && off_a_surface <= curve.deviation + 1e-3 * TOL) {
            return Err(format!(
                "WRONG: a curve {off_a_surface:e} from a surface, its deviation {:e}",
                curve.deviation
            ));
        }
        if !(within.contains(first, 2.0 * TOL) && within.contains(last, 2.0 * TOL)) {
            return Err("WRONG: a curve that ends outside the box".to_string());
        }
        if curve.closed && !(first.distance(last) <= 1e-3 * TOL) {
            return Err("WRONG: a closed curve that does not close".to_string());
        }
        for (a, b) in angles.iter().zip(angles.iter().skip(1)) {
            let turn = (b - a + PI).rem_euclid(TAU) - PI;
            let bin = ((0.5 * (a + b)).rem_euclid(TAU) / TAU * BINS as f64) as usize;
            passed[bin.min(BINS - 1)] += turn.abs();
        }
    }
    if !(worst <= ALONG * TOL) {
        return Err(format!(
            "WRONG: a point of a curve {:.3} tolerances from the crossing",
            worst / TOL
        ));
    }
    // A loop that reaches round the part or over the cone's apex is not
    // one loop about its middle: held to the crossing, and no further.
    let (ku, kz) = case.half_sizes();
    if !(ku < 0.5 && (!case.cone || kz < 5.0)) {
        return Ok(worst / TOL);
    }
    let width = TAU / BINS as f64;
    let (lo, hi) = within.corners().unwrap();
    let margin = Vec3::new(8.0 * TOL, 8.0 * TOL, 8.0 * TOL);
    let well_inside = Bbox3::from_points([lo + margin, hi - margin]);
    let there: Vec<bool> = (0..BINS)
        .map(|bin| {
            case.loop_point((bin as f64 + 0.5) * width, within)
                .is_some_and(|q| well_inside.contains(q, 0.0))
        })
        .collect();
    let (mut asked, mut missed, mut twice) = (0, 0, 0);
    for bin in 0..BINS {
        // The loop inside the box in this direction and in those beside.
        if !(there[bin] && there[(bin + 1) % BINS] && there[(bin + BINS - 1) % BINS]) {
            continue;
        }
        asked += 1;
        let near = passed[bin] + passed[(bin + 1) % BINS] + passed[(bin + BINS - 1) % BINS];
        missed += usize::from(near < 0.5 * width);
        twice += usize::from(passed[bin] > 1.6 * width);
    }
    if asked == BINS && !(m.curves.len() == 1 && m.curves[0].closed) {
        return Err(format!(
            "WRONG: the loop is inside the box, and {} curves (closed: {:?})",
            m.curves.len(),
            m.curves.iter().map(|c| c.closed).collect::<Vec<_>>()
        ));
    }
    if missed > 0 || twice > 0 {
        return Err(format!(
            "WRONG: of {asked} directions in which the loop is inside the box, {missed} are passed by no curve and {twice} more than once"
        ));
    }
    Ok(worst / TOL)
}

/// The review's sweep: radii 1e3 to 1e6, from 10 tolerances outside the
/// side to 1e5 inside.
fn cases() -> Vec<Case> {
    let mut grid: Vec<(f64, f64)> = Vec::new();
    for radius in [1e3, 1e4, 3e4, 1e5, 1e6] {
        for sunk in [
            -10.0, -3.0, -1.5, -0.3, 0.3, 1.5, 3.0, 10.0, 30.0, 100.0, 1000.0, 1e5,
        ] {
            grid.push((radius, sunk));
        }
    }
    for radius in [2e4, 3e4, 5e4, 7e4, 1e5, 2e5, 3e5] {
        for sunk in [3.0, 5.0, 10.0, 20.0, 30.0, 50.0, 100.0, 200.0, 300.0] {
            if !grid.contains(&(radius, sunk)) {
                grid.push((radius, sunk));
            }
        }
    }
    for sunk in [4.0, 4.5, 5.5, 6.0, 7.0, 8.0] {
        grid.push((2e4, sunk));
    }
    [false, true]
        .into_iter()
        .flat_map(|cone| {
            grid.iter()
                .map(move |&(radius, sunk)| Case { cone, radius, sunk })
        })
        .collect()
}

/// The exact distance is what it says: against a placement in which the
/// plain difference loses nothing (the centre on the x axis at `r + e`:
/// `q.x − c.x` is `(q.x − e) − r`, both differences of numbers within a
/// factor of two of each other, exact), at radii from 10 to 1e7.
#[test]
fn the_exact_distance_of_the_test_is_the_placements() {
    for r in [10.0, 1e3, 1e5, 1e7] {
        let c = Vec3::new(r + 3.999_996_3, 0.0, 0.7);
        let e = c.x - r;
        for (x, y, z) in [(4.0, 0.0, 0.7), (3.9999, 0.03, 1.1), (3.99, -0.2, 0.3)] {
            // (s − r)² + y² + (z − 0.7)² − r², the r² gone by hand.
            let s: f64 = x - e;
            let dz: f64 = z - 0.7;
            let squares = s * s - 2.0 * s * r + y * y + dz * dz;
            let q = Vec3::new(x, y, z);
            let by_hand = squares / (q.distance(c) + r);
            let exact = ball_distance(q, c, r);
            assert!(
                (exact - by_hand).abs() <= 1e-15 + 1e-12 * by_hand.abs(),
                "radius {r:e} at {q:?}: {exact:e} and {by_hand:e}"
            );
        }
    }
}

/// Every pair of the sweep, in both boxes a boolean asks it in: right, or
/// refused in words; none wrong. The pairs refused are listed, and held
/// to their number (one more is a regression, one fewer a gain to lock
/// in): four today, the two pairs whose loop ends on the cone's apex (a
/// ball of 1e5 sunk 200 tolerances and one of 2e5 sunk 100: the loop
/// reaches 6.32 up the side from its middle, which is where the apex
/// is), in both boxes, "surfaces that touch along a curve, or with more
/// than second-order contact, without a closed form".
#[test]
fn a_ball_far_larger_than_the_part_meets_its_side_as_it_lies() {
    let mut wrong = Vec::new();
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut right, mut asked) = (0, 0);
    let (mut furthest, mut dearest) = ((0.0_f64, String::new()), (0_u64, String::new()));
    let started = craft3d_geom::intersect::march_work();
    for case in cases() {
        for (which, within) in case.boxes().iter().enumerate() {
            asked += 1;
            let before = craft3d_geom::intersect::march_work();
            let name = format!(
                "{} {:e} sunk {} in the {} box",
                if case.cone { "cone" } else { "cylinder" },
                case.radius,
                case.sunk,
                if which == 0 { "large" } else { "small" }
            );
            let said = verdict(&case, within);
            let took = craft3d_geom::intersect::march_work() - before;
            if took > dearest.0 {
                dearest = (took, name.clone());
            }
            match said {
                Ok(off) => {
                    right += 1;
                    if off > furthest.0 {
                        furthest = (off, name);
                    }
                }
                Err(why) if why.starts_with("refused") => {
                    refused.entry(why).or_default().push(name);
                }
                Err(why) => wrong.push(format!("{name}: {why}")),
            }
        }
    }
    let evaluations = craft3d_geom::intersect::march_work() - started;
    let not_told: usize = refused.values().map(Vec::len).sum();
    println!(
        "{asked} questions: {right} right, {not_told} refused, {} wrong; {evaluations} evaluations, at most {} ({}); a curve at most {:.3} tolerances from the crossing ({})",
        wrong.len(),
        dearest.0,
        dearest.1,
        furthest.0,
        furthest.1
    );
    for line in &wrong {
        println!("{line}");
    }
    for (why, names) in &refused {
        println!("{} {why}:", names.len());
        for name in names {
            println!("    {name}");
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert_eq!(not_told, REFUSED, "refused: see the list printed");
    // What the sweep may cost (134 million, 2026-11-21): with a cone
    // bounded by its width in the whole box it took 2 582 million, and 45
    // of its questions ran into the cap on evaluations.
    assert!(
        evaluations <= EVALUATIONS,
        "{evaluations} evaluations, at most {EVALUATIONS}"
    );
}

/// The most evaluations the sweep's questions may take together.
const EVALUATIONS: u64 = 200_000_000;

/// How many of the sweep's questions are refused today (owed).
const REFUSED: usize = 4;
