//! The review of `check/near-crossing-twice` (2026-11-25): a crossing point
//! read where the surfaces are a little apart, its zone, and the two rules
//! the branch adds there (a seed inside the zone along an arm a branch
//! already comes into the point along is on that branch; a run that goes
//! round the point onto such an arm ends at the point; docs/design/
//! booleans.md › *A stretch given twice beside a crossing point the
//! surfaces are a little apart at*).
//!
//! Every pair is asked of the marcher (`meet_surfaces_numeric`) and held to
//! its crossing as known in closed form, by four oracles that know nothing
//! of the march:
//!
//! - `held`, as the review of the fold holds a crossing (copied from
//!   `meet_fold_review.rs`): every curve on both surfaces and in the box,
//!   on the crossing, as long as the crossing away from the points where
//!   its curves cross or pass close, and every place of the crossing there
//!   on a curve;
//! - the ends: every end of an open curve is within two tolerances of such
//!   a point or of a side of the box, and at such a point well inside the
//!   box an even number of ends meet (the crossing inside the box is made
//!   of closed loops, so where curves end inside it they end in pairs);
//! - the whole length: all the curves together as long as the whole
//!   crossing in the box, nothing left out about the points, within 32
//!   tolerances, more by at most four times how near the crossing passes
//!   each point (a curve drawn straight into a point the crossing passes
//!   at a distance `c` is longer than the crossing there by at most `2c`
//!   for each of the two curves that pass it);
//! - the exits (added 2026-11-26, with the repair of the boxes whose side
//!   passes the point): where the crossing leaves the box within the
//!   radius `held` leaves out about such a point, a curve ends on the box
//!   within that radius of the place.
//!
//! The families: Viviani's figure and Steinmetz's ellipses opened by gaps
//! from a millionth to a hundredth of a radius, either way; two cylinders
//! of nearly one radius whose axes cross at 30, 45 and 60 degrees; a plane
//! beside the inner equator of tori of five shapes (arms from 11 to 120
//! degrees apart; the torus's seam through the point and away from it); a
//! cone and a cylinder that both nearly touch one ball (Monge's two
//! conics, two crossing points); the opened figures in boxes so large that
//! the zone of a point holds the whole figure and the other point, and in
//! boxes whose side passes through the zone. Each answer is written out
//! with a digest of its bits, so that two builds can be compared pair by
//! pair.
//!
//! Measured (2026-11-25; `main` ed93bafc with these tests, Check
//! 45711338836; the branch, Check 45711343773): of 4 456 pairs (all but
//! the turned boxes through the zone, rewritten after those runs), 632
//! were not the crossing or refused on `main`, at gaps of one and three
//! ten-thousandths of a radius only; with the branch 59, every one of them
//! not the crossing on `main` either. The bits of 582 answers moved, all
//! at those two gaps, every one of them an answer that was not the
//! crossing or refused on `main`; no answer that was the crossing on
//! `main` moved. Among them the 26 of Viviani's opened outwards that the
//! review of the fold took for the crossing: on `main` each has a curve
//! that ends 80.0 tolerances from the point, inside the box, where the
//! crossing (one closed loop) has no end; with the branch every end is on
//! the point or the box. What is wrong with the branch is wrong on `main`
//! (four tests below, ignored as owed). The turned boxes through the zone,
//! rewritten: 87 of 168 not the crossing or refused on `main` (Check
//! 45713376999), 18 with the branch (Check 45713380970), each of them
//! wrong on `main` too.
//!
//! The boxes through the zone, repaired (2026-11-26, docs/design/
//! booleans.md › *A box whose side passes a crossing point the surfaces
//! are a little apart at*): every one of the 336 the crossing, held to the
//! four oracles; the two tests no longer ignored. The other two stay owed.
//!
//! The review of that repair (2026-11-26) adds a fifth oracle, the converse
//! of the exits (`exits_counted`): about such a point clear of the box's
//! sides, curves end on the box as often as the crossing leaves it there.
//! Held to it, 17 of the 336 boxes through the zone are not the crossing
//! with the repair (each wrong on the base too), and two more families
//! through the zone (fine insets near where the crossing turns; tori and a
//! cone) have 93 of 768 and 20 of 360 so (156 and 79 on the base): three
//! tests, ignored as owed. No answer the five take for the crossing on the
//! base is otherwise with the repair (Checks 45763928388, 45763931681).

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

use common::{cone, cylinder, distance, normal_at, plane, sphere, torus};
use craft3d_geom::intersect::{Meeting, MeetingCurve, SurfaceMeeting, meet_surfaces_numeric};
use craft3d_geom::{GeomError, Surface};
use craft3d_math::{Bbox3, Frame, Portable, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// A line on the test's output that the harness does not hold back.
fn say(line: &str) {
    let _ = std::io::Write::write_all(&mut std::io::stderr(), format!("{line}\n").as_bytes());
}

/// The four scales a pair is asked at.
const SCALES_FOUR: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// Where a box is put off the place it is about, in its half sides.
const OFF: Vec3 = Vec3::new(0.13, -0.21, 0.37);

/// The tolerance at scale `k`.
fn tolerance(k: f64) -> Tolerance {
    Tolerance::new(1e-6 * k, 1e-9).unwrap()
}

/// Where a pair's places go: scaled by `k` about the origin of the
/// coordinates it is written in, then put in `frame`.
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

    fn frame_of(&self, origin: Vec3, z: Vec3, x: Vec3) -> Frame {
        Frame::new(self.point(origin), self.vector(z), self.vector(x)).unwrap()
    }

    /// The box along the world's axes that holds the cube of half side
    /// `half` about `centre`, both in the pair's own coordinates.
    fn cube(&self, centre: Vec3, half: f64) -> Bbox3 {
        let mut corners = Vec::new();
        for x in [-half, half] {
            for y in [-half, half] {
                for z in [-half, half] {
                    corners.push(self.point(centre + Vec3::new(x, y, z)));
                }
            }
        }
        Bbox3::from_points(corners)
    }
}

/// Upright (the pair as written, scaled) and turned (askew to the world's
/// axes and moved off its origin).
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

/// The places of a piece of the crossing along a parameter.
type Place = dyn Fn(f64) -> Vec3;

/// A piece of the crossing known in closed form: its places (in the
/// world) over `from..to` of a parameter that runs one way along it.
struct Sheet {
    at: Box<Place>,
    from: f64,
    to: f64,
}

/// A pair, the box and tolerance it is asked in, and its crossing as
/// known without the marcher.
struct Pair {
    what: String,
    a: Surface,
    b: Surface,
    within: Bbox3,
    tol: Tolerance,
    sheets: Vec<Sheet>,
    /// Where curves of the crossing cross or pass close to each other:
    /// within `keep_off_radius` of these no place of the crossing is
    /// asked to be on a curve.
    keep_off: Vec<Vec3>,
    keep_off_radius: f64,
}

impl Pair {
    /// Whether `p` is further than `keep_off_radius` from every place where
    /// curves of the crossing cross or pass close to each other.
    fn clear(&self, p: Vec3) -> bool {
        self.keep_off
            .iter()
            .all(|c| c.distance(p) > self.keep_off_radius)
    }
}

/// The stretches of a sheet's parameter over which its places lie in the
/// box grown by `margin`, read at `n + 1` places, each widened by one
/// step before it.
fn stretches(sheet: &Sheet, within: &Bbox3, margin: f64, n: usize) -> Vec<(f64, f64)> {
    let step = (sheet.to - sheet.from) / n as f64;
    let mut found = Vec::new();
    let mut open: Option<f64> = None;
    for i in 0..=n {
        let s = sheet.from + step * i as f64;
        let inside = within.contains((sheet.at)(s), margin);
        match (inside, open) {
            (true, None) => open = Some((s - step).max(sheet.from)),
            (false, Some(first)) => {
                found.push((first, s));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(first) = open {
        found.push((first, sheet.to));
    }
    found
}

/// How long a sheet is inside the box and where `clear` holds: `n` chords
/// per stretch, those with both ends inside whole, those across a side of
/// the box up to it (found by halving), none with an end where `clear`
/// does not hold.
fn length_inside(
    sheet: &Sheet,
    within: &Bbox3,
    stretches: &[(f64, f64)],
    n: usize,
    clear: &dyn Fn(Vec3) -> bool,
) -> f64 {
    let mut length = 0.0;
    for &(s0, s1) in stretches {
        let step = (s1 - s0) / n as f64;
        let mut before = (s0, (sheet.at)(s0));
        let mut was_in = within.contains(before.1, 0.0);
        for i in 1..=n {
            let s = s0 + step * i as f64;
            let p = (sheet.at)(s);
            let is_in = within.contains(p, 0.0);
            let clear_here = clear(before.1) && clear(p);
            if clear_here && was_in && is_in {
                length += before.1.distance(p);
            } else if clear_here && was_in != is_in {
                let (mut inner, mut outer) = if was_in { (before.0, s) } else { (s, before.0) };
                for _ in 0..60 {
                    let mid = 0.5 * (inner + outer);
                    if within.contains((sheet.at)(mid), 0.0) {
                        inner = mid;
                    } else {
                        outer = mid;
                    }
                }
                let side = (sheet.at)(inner);
                length += if was_in {
                    before.1.distance(side)
                } else {
                    side.distance(p)
                };
            }
            before = (s, p);
            was_in = is_in;
        }
    }
    length
}

/// Places along a parameter from `from` to `to`, read first at `n + 1`
/// places (`rows`): the distance from a place to them is found about the
/// nearest row by golden sections.
struct Along<F: Fn(f64) -> Vec3> {
    at: F,
    from: f64,
    to: f64,
    rows: Vec<Vec3>,
}

impl<F: Fn(f64) -> Vec3> Along<F> {
    fn new(at: F, from: f64, to: f64, n: usize) -> Self {
        let rows = (0..=n)
            .map(|i| at(from + (to - from) * i as f64 / n as f64))
            .collect();
        Self { at, from, to, rows }
    }

    /// How far the nearest row is from `q`, and which it is.
    fn nearest_row(&self, q: Vec3) -> (f64, usize) {
        self.rows
            .iter()
            .enumerate()
            .map(|(i, p)| (p.distance(q), i))
            .fold((f64::INFINITY, 0), |best, next| {
                if next.0 < best.0 { next } else { best }
            })
    }

    /// How far `q` is from the places between the rows either side of row
    /// `i`.
    fn about(&self, i: usize, q: Vec3) -> f64 {
        let n = (self.rows.len() - 1) as f64;
        let step = (self.to - self.from) / n;
        let mut lo = (self.from + step * (i as f64 - 1.0)).max(self.from);
        let mut hi = (self.from + step * (i as f64 + 1.0)).min(self.to);
        for _ in 0..60 {
            let (m1, m2) = (lo + (hi - lo) * 0.381_966, lo + (hi - lo) * 0.618_034);
            if (self.at)(m1).distance(q) <= (self.at)(m2).distance(q) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        (self.at)(0.5 * (lo + hi))
            .distance(q)
            .min(self.rows[i].distance(q))
    }
}

/// How far `q` is from the nearest of several runs of places.
fn distance_to<F: Fn(f64) -> Vec3>(runs: &[Along<F>], q: Vec3) -> f64 {
    let rows: Vec<(f64, usize)> = runs.iter().map(|r| r.nearest_row(q)).collect();
    let best = rows.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    runs.iter()
        .zip(&rows)
        .filter(|(_, row)| row.0 <= 2.0 * best)
        .map(|(run, row)| run.about(row.1, q))
        .fold(f64::INFINITY, f64::min)
}

/// The sine of the angle between the two surfaces at the places nearest
/// `p` (1 where a normal is not known).
fn sine(pair: &Pair, p: Vec3) -> f64 {
    match (normal_at(&pair.a, p), normal_at(&pair.b, p)) {
        (Some(na), Some(nb)) => na.cross(nb).length(),
        _ => 1.0,
    }
}

/// Whether what the marcher answered is the crossing: every curve on both
/// surfaces within its deviation and inside the box; every point of it on
/// the crossing (within four tolerances, and what being within its
/// deviation of both surfaces allows where they cross at a shallow
/// angle); all of them together as long as the crossing is inside the
/// box; and every place of the crossing inside the box (away from where
/// its curves cross or pass close) on a curve. The reason where not.
fn held(pair: &Pair, m: &Meeting) -> Result<(), String> {
    let t = pair.tol.linear;
    if m.curves.is_empty() {
        return Err(format!("no curve ({} points)", m.points.len()));
    }
    let within = &pair.within;
    let mut known: Vec<Along<&Place>> = Vec::new();
    let mut pieces: Vec<(usize, f64, f64)> = Vec::new();
    let mut length_known = 0.0;
    for (n, sheet) in pair.sheets.iter().enumerate() {
        let found = stretches(sheet, within, 8.0 * t, 1 << 16);
        length_known += length_inside(sheet, within, &found, 1 << 15, &|p: Vec3| pair.clear(p));
        for &(s0, s1) in &found {
            known.push(Along::new(&*sheet.at, s0, s1, 2048));
            pieces.push((n, s0, s1));
        }
    }
    if known.is_empty() {
        return Err("the crossing as known is not in the box".to_string());
    }
    let deviation = m.curves.iter().map(|c| c.deviation).fold(0.0, f64::max);
    let mut length = 0.0;
    for (n, curve) in m.curves.iter().enumerate() {
        if !(curve.deviation >= 0.0 && curve.deviation <= t) {
            return Err(format!("curve {n}: deviation {}", curve.deviation));
        }
        let (t0, t1) = curve.range;
        if !(t0 < t1) {
            return Err(format!("curve {n}: range {:?}", curve.range));
        }
        let at = |i: u32, of: u32| {
            curve
                .curve
                .eval(t0 + (t1 - t0) * f64::from(i) / f64::from(of))
        };
        let mut before = at(0, 1 << 14);
        for i in 1..=(1 << 14) {
            let p = at(i, 1 << 14);
            if pair.clear(before) && pair.clear(p) {
                length += before.distance(p);
            }
            before = p;
        }
        for i in 0..=200 {
            let p = at(i, 200);
            let (da, db) = (distance(&pair.a, p), distance(&pair.b, p));
            // (Where curves of the crossing pass close, the march may
            // read a point where they cross, the surfaces a little apart
            // there, and end its curves on it: that far off one surface,
            // within the tolerance.)
            let bound = if pair.clear(p) {
                curve.deviation + 1e-3 * t
            } else {
                t
            };
            if !(da <= bound && db <= bound) {
                return Err(format!(
                    "curve {n} is {} and {} tolerances off the surfaces at {p:?}",
                    da / t,
                    db / t
                ));
            }
            if !within.contains(p, 2.0 * t) {
                return Err(format!("curve {n} leaves the box at {p:?}"));
            }
            let off = distance_to(&known, p);
            let room = 4.0 * t + 2.0 * curve.deviation / sine(pair, p).max(1e-9);
            if !(off <= room) {
                return Err(format!(
                    "curve {n} is {} tolerances off the crossing at {p:?} (room {})",
                    off / t,
                    room / t
                ));
            }
        }
    }
    if !((length - length_known).abs() <= 32.0 * t + 1e-5 * length_known) {
        return Err(format!(
            "the curves are {} tolerances long, the crossing in the box {} (away from where its curves cross or pass close); {}",
            length / t,
            length_known / t,
            described(pair, m)
        ));
    }
    let answered = runs_of(m);
    for &(n, s0, s1) in &pieces {
        let sheet = &pair.sheets[n];
        for i in 0..=96 {
            let q = (sheet.at)(s0 + (s1 - s0) * f64::from(i) / 96.0);
            if !within.contains(q, -4.0 * t) || !pair.clear(q) {
                continue;
            }
            let off = distance_to(&answered, q);
            let room = 2.0 * t + 2.0 * deviation / sine(pair, q).max(1e-9);
            if !(off <= room) {
                return Err(format!(
                    "the crossing at {q:?} is {} tolerances from every curve (room {})",
                    off / t,
                    room / t
                ));
            }
        }
    }
    Ok(())
}

/// The curves of an answer as runs of places.
fn runs_of(m: &Meeting) -> Vec<Along<impl Fn(f64) -> Vec3>> {
    m.curves
        .iter()
        .map(|c| Along::new(move |s: f64| c.curve.eval(s), c.range.0, c.range.1, 2048))
        .collect()
}

/// Each curve of an answer, for a line about one that is not the
/// crossing: how long it is, whether it closes, its ends, and how much of
/// it (away from where curves of the crossing cross or pass close) lies
/// within two tolerances of another curve of the answer.
fn described(pair: &Pair, m: &Meeting) -> String {
    let t = pair.tol.linear;
    let runs = runs_of(m);
    let mut lines = Vec::new();
    for (n, curve) in m.curves.iter().enumerate() {
        let (t0, t1) = curve.range;
        let at = |i: u32| curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 1000.0);
        let (mut length, mut twice) = (0.0, 0.0);
        let mut before = at(0);
        for i in 1..=1000 {
            let p = at(i);
            let step = before.distance(p);
            length += step;
            let other = runs
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != n)
                .map(|(_, r)| r.about(r.nearest_row(p).1, p))
                .fold(f64::INFINITY, f64::min);
            if other <= 2.0 * t && pair.clear(p) {
                twice += step;
            }
            before = p;
        }
        lines.push(format!(
            "curve {n} {} tolerances long, closed {}, from {:?} to {:?}, {} tolerances of it on another curve",
            length / t,
            curve.closed,
            at(0),
            at(1000),
            twice / t
        ));
    }
    lines.join("; ")
}

/// A ball of radius `2a` about the origin and a cylinder of radius
/// `a' = a(1 + η)` along z through `(a, 0, 0)` (`a` 1e-3): Viviani's
/// figure of eight opened at its crossing point `(2a, 0, 0)`. About the
/// cylinder's axis at the angle `θ` the crossing is
/// `(a + a' cos θ, a' sin θ, ±√g)`, `g = a²(4(1 + η) sin²(θ/2) − η(4 + η))`:
/// for `η < 0` two loops that pass about `4a√(−η)` apart beside that
/// point; for `η > 0` one curve that turns on a radius of about `2a√η`
/// where it crosses `z = 0` at `θ0` and `2π − θ0` (`g(θ0) = 0`), along
/// `θ = θ0 + (π − θ0)(1 − cos s)`. The cube of half side `size · a` about
/// that point.
fn ball_and_tube_opened(placed: Placed, eta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let a = 1e-3;
    let tube = a * (1.0 + eta);
    let g = move |theta: f64| {
        let h = (0.5 * theta).psin();
        (a * a * (4.0 * (1.0 + eta) * h * h - eta * (4.0 + eta))).max(0.0)
    };
    let theta0 = if eta > 0.0 {
        2.0 * (eta * (4.0 + eta) / (4.0 * (1.0 + eta))).sqrt().pasin()
    } else {
        0.0
    };
    let theta_of = move |s: f64| {
        if eta > 0.0 {
            theta0 + (PI - theta0) * (1.0 - s.pcos())
        } else {
            s
        }
    };
    let (from, to) = if eta > 0.0 { (0.0, PI) } else { (-PI, PI) };
    let side = move |sign: f64| -> Sheet {
        Sheet {
            at: Box::new(move |s: f64| {
                let theta = theta_of(s);
                placed.point(Vec3::new(
                    a + tube * theta.pcos(),
                    tube * theta.psin(),
                    sign * g(theta).sqrt(),
                ))
            }),
            from,
            to,
        }
    };
    let half = size * a;
    let within = placed.cube(Vec3::new(2.0 * a, 0.0, 0.0) + OFF * half, half);
    Pair {
        what: format!("ball and tube opened at {k}, {how}: eta {eta}, cube of half side {half}"),
        a: sphere(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), 2.0 * a * k),
        b: cylinder(
            placed.frame_of(Vec3::new(a, 0.0, 0.0), Vec3::Z, Vec3::X),
            tube * k,
        ),
        within,
        tol: tolerance(k),
        sheets: vec![side(1.0), side(-1.0)],
        keep_off: vec![placed.point(Vec3::new(2.0 * a, 0.0, 0.0))],
        keep_off_radius: (0.05 * a * k).max(within.diagonal() / 1600.0),
    }
}

/// Two cylinders at right angles, of radius `r` (1e-3, along z) and
/// `R = r(1 + η)` (along x), through the origin: Steinmetz's two ellipses
/// opened at their crossing points `(0, ±r, 0)`. For `η > 0` two loops
/// about the first, `z = ±r√(cos²θ + η(2 + η))` at its angle `θ`; for
/// `η < 0` two loops about the second, `x = ±R√(cos²φ + μ(2 + μ))` at its
/// angle `φ` (`r = R(1 + μ)`). They pass about
/// `2r√(2|η|)` apart beside those points. The cube of half side
/// `size · r` about `(0, r, 0)`.
fn tubes_opened(placed: Placed, eta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let r = 1e-3;
    let big = r * (1.0 + eta);
    let mu = -eta / (1.0 + eta);
    let side = move |sign: f64| -> Sheet {
        Sheet {
            at: Box::new(move |s: f64| {
                let (sn, cs) = (s.psin(), s.pcos());
                placed.point(if eta > 0.0 {
                    Vec3::new(
                        r * cs,
                        r * sn,
                        sign * r * (cs * cs + eta * (2.0 + eta)).sqrt(),
                    )
                } else {
                    Vec3::new(
                        sign * big * (cs * cs + mu * (2.0 + mu)).sqrt(),
                        big * sn,
                        big * cs,
                    )
                })
            }),
            from: -PI,
            to: PI,
        }
    };
    let half = size * r;
    let within = placed.cube(Vec3::new(0.0, r, 0.0) + OFF * half, half);
    Pair {
        what: format!("two tubes opened at {k}, {how}: eta {eta}, cube of half side {half}"),
        a: cylinder(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), r * k),
        b: cylinder(placed.frame_of(Vec3::ZERO, Vec3::X, Vec3::Y), big * k),
        within,
        tol: tolerance(k),
        sheets: vec![side(1.0), side(-1.0)],
        keep_off: vec![
            placed.point(Vec3::new(0.0, r, 0.0)),
            placed.point(Vec3::new(0.0, -r, 0.0)),
        ],
        keep_off_radius: (0.05 * r * k).max(within.diagonal() / 1600.0),
    }
}

/// `count` sizes from `least`, each `2^(1/2)` times the last.
fn sizes(least: f64, count: i32) -> Vec<f64> {
    (0..count)
        .map(|j| least * 2f64.ppowf(f64::from(j) / 2.0))
        .collect()
}

/// Every pair of a family at the four scales, upright and turned.
fn every(make: impl Fn(Placed, &'static str, &mut Vec<Pair>)) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for scale in SCALES_FOUR {
        for (how, placed) in placings(scale) {
            make(placed, how, &mut pairs);
        }
    }
    pairs
}

/// The crossing where it is `place(φ, ±√g(φ))` for `φ` over a period from
/// `from`: two sheets over the whole period where `g` is positive
/// throughout; else two over each stretch where it is, from one zero to
/// the next, along `φ = middle − half · cos s` for `s` from 0 to π (at a
/// simple zero the square root goes as `s`, so the places are spread
/// evenly along the curve there). `g` is read at 65 536 places, enough for
/// a stretch where it is negative two thousandths of a radian wide.
fn loops<G, P>(g: G, place: P, from: f64) -> Vec<Sheet>
where
    G: Fn(f64) -> f64 + Copy + 'static,
    P: Fn(f64, f64) -> Vec3 + Copy + 'static,
{
    let n = 1_u32 << 16;
    let step = 2.0 * PI / f64::from(n);
    let mut lowest = (0_u32, f64::INFINITY);
    for i in 0..n {
        let value = g(from + step * f64::from(i));
        if value < lowest.1 {
            lowest = (i, value);
        }
    }
    let mut sheets = Vec::new();
    if lowest.1 > 0.0 {
        for sign in [1.0, -1.0] {
            sheets.push(Sheet {
                at: Box::new(move |phi: f64| place(phi, sign * g(phi).max(0.0).sqrt())),
                from,
                to: from + 2.0 * PI,
            });
        }
        return sheets;
    }
    // From the lowest place, where `g` is not positive: no stretch runs
    // over the end of the period.
    let start = from + step * f64::from(lowest.0);
    let phi = move |i: u32| start + step * f64::from(i);
    let zero = move |mut inside: f64, mut outside: f64| {
        for _ in 0..100 {
            let mid = 0.5 * (inside + outside);
            if g(mid) > 0.0 {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        inside
    };
    let mut i = 1_u32;
    while i < n {
        if !(g(phi(i)) > 0.0) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && g(phi(j)) > 0.0 {
            j += 1;
        }
        let (lo, hi) = (zero(phi(i), phi(i - 1)), zero(phi(j - 1), phi(j)));
        let (middle, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
        for sign in [1.0, -1.0] {
            sheets.push(Sheet {
                at: Box::new(move |s: f64| {
                    let p = middle - half * s.pcos();
                    place(p, sign * g(p).max(0.0).sqrt())
                }),
                from: 0.0,
                to: PI,
            });
        }
        i = j;
    }
    sheets
}

/// Two cylinders whose axes cross at the origin at the angle `γ`: the first
/// of radius `r` (1e-3) along z, the second of radius `r(1 + η)` along
/// `(sin γ, 0, cos γ)`; for `η = 0` two ellipses that cross each other at
/// `(0, ±r, 0)`, where the cylinders touch. Walked round the narrower one
/// (its axis `a`, its angle `θ` from `e₁` towards `e₂`, radius `ρ₁`; the
/// other's axis `d`, radius `ρ₂`), the crossing is the place at
/// `w = (α β ± √(β² + (1 − α²)(ρ₂² − ρ₁²))) / (1 − α²)` along `a`, with
/// `α = a · d` and `β = ρ₁ (cos θ e₁ + sin θ e₂) · d`: two loops that pass
/// close beside those points. The cube of half side `size · r` about
/// `(0, r, 0)`.
fn tubes_at_opened(placed: Placed, gamma: f64, eta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let r = 1e-3;
    let big = r * (1.0 + eta);
    let (sg, cg) = (gamma.psin(), gamma.pcos());
    let second = Vec3::new(sg, 0.0, cg);
    let (axis, e1, e2, narrow, wide, other) = if eta >= 0.0 {
        (Vec3::Z, Vec3::X, Vec3::Y, r, big, second)
    } else {
        (second, Vec3::Y, Vec3::new(-cg, 0.0, sg), big, r, Vec3::Z)
    };
    let alpha = axis.dot(other);
    let (d1, d2) = (e1.dot(other), e2.dot(other));
    let squeeze = 1.0 - alpha * alpha;
    let side = move |sign: f64| -> Sheet {
        Sheet {
            at: Box::new(move |s: f64| {
                let (sn, cs) = (s.psin(), s.pcos());
                let beta = narrow * (cs * d1 + sn * d2);
                let disc = (beta * beta + squeeze * (wide * wide - narrow * narrow)).max(0.0);
                let w = (alpha * beta + sign * disc.sqrt()) / squeeze;
                placed.point(e1 * (narrow * cs) + e2 * (narrow * sn) + axis * w)
            }),
            from: -PI,
            to: PI,
        }
    };
    let half = size * r;
    let within = placed.cube(Vec3::new(0.0, r, 0.0) + OFF * half, half);
    Pair {
        what: format!(
            "two tubes at {gamma} rad opened at {k}, {how}: eta {eta}, cube of half side {half}"
        ),
        a: cylinder(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), r * k),
        b: cylinder(placed.frame_of(Vec3::ZERO, second, Vec3::Y), big * k),
        within,
        tol: tolerance(k),
        sheets: vec![side(1.0), side(-1.0)],
        keep_off: vec![
            placed.point(Vec3::new(0.0, r, 0.0)),
            placed.point(Vec3::new(0.0, -r, 0.0)),
        ],
        keep_off_radius: (0.05 * r * k).max(within.diagonal() / 1600.0),
    }
}

/// A torus of minor radius `ρ` (1e-3) and major radius `R = q ρ` about z
/// (its seam along `seam`), and the plane `x = x₀ = R − ρ + η ρ`: beside
/// the torus's inner equator at `(R − ρ, 0, 0)`, where the plane
/// `x = R − ρ` touches it and crosses it in a figure of eight whose arms
/// leave the point at `atan √(q − 1)` either side of z. At the torus's
/// angle `v` round its tube the crossing is
/// `(x₀, ±√((R + ρ cos v)² − x₀²), ρ sin v)`: for `η < 0` two loops that
/// pass beside the point, for `η > 0` one curve that turns beside it twice.
/// The cube of half side `size · ρ` about the point.
fn torus_and_plane(placed: Placed, q: f64, seam: Vec3, eta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let rho = 1e-3;
    let big = q * rho;
    let x0 = big - rho + eta * rho;
    let g = move |v: f64| {
        let w = big + rho * v.pcos();
        w * w - x0 * x0
    };
    let sheets = loops(
        g,
        move |v: f64, y: f64| placed.point(Vec3::new(x0, y, rho * v.psin())),
        -PI,
    );
    let at = Vec3::new(big - rho, 0.0, 0.0);
    let half = size * rho;
    let within = placed.cube(at + OFF * half, half);
    Pair {
        what: format!(
            "torus {q} seam ({}, {}) and plane at {k}, {how}: eta {eta}, cube of half side {half}",
            seam.x, seam.y
        ),
        a: plane(placed.frame_of(Vec3::new(x0, 0.0, 0.0), Vec3::X, Vec3::Y)),
        b: torus(placed.frame_of(Vec3::ZERO, Vec3::Z, seam), big * k, rho * k),
        within,
        tol: tolerance(k),
        sheets,
        keep_off: vec![placed.point(Vec3::new(big - rho + 0.5 * eta * rho, 0.0, 0.0))],
        keep_off_radius: (0.05 * rho * k).max(within.diagonal() / 1600.0),
    }
}

/// A cylinder of radius `ρ = s(1 + η)` along x through the origin (`s`
/// 1e-3) and the cone about z narrowing upwards at the half-angle `α` with
/// its apex at `h = s / sin α`: for `η = 0` both touch the ball of radius
/// `s` about the origin and cross in two conics (Monge), which cross each
/// other where both touch it, `(0, ±s cos α, s sin α)`. At the cylinder's
/// angle `φ` the crossing is `(±√g, ρ cos φ, ρ sin φ)`,
/// `g = ((h − ρ sin φ) tan α)² − ρ² cos² φ`. The cube of half side
/// `size · s` about the first of those points.
fn cone_and_cylinder(placed: Placed, alpha: f64, eta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let s0 = 1e-3;
    let rho = s0 * (1.0 + eta);
    let (sa, ca) = (alpha.psin(), alpha.pcos());
    let slope = sa / ca;
    let h = s0 / sa;
    let g = move |phi: f64| {
        let w = (h - rho * phi.psin()) * slope;
        let c = rho * phi.pcos();
        w * w - c * c
    };
    let sheets = loops(
        g,
        move |phi: f64, x: f64| placed.point(Vec3::new(x, rho * phi.pcos(), rho * phi.psin())),
        -PI,
    );
    let at = Vec3::new(0.0, s0 * ca, s0 * sa);
    let half = size * s0;
    let within = placed.cube(at + OFF * half, half);
    Pair {
        what: format!(
            "cone {alpha} rad and cylinder at {k}, {how}: eta {eta}, cube of half side {half}"
        ),
        a: cylinder(placed.frame_of(Vec3::ZERO, Vec3::X, Vec3::Y), rho * k),
        b: cone(
            placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X),
            h * slope * k,
            -alpha,
        ),
        within,
        tol: tolerance(k),
        sheets,
        keep_off: vec![
            placed.point(at),
            placed.point(Vec3::new(0.0, -s0 * ca, s0 * sa)),
        ],
        keep_off_radius: (0.05 * s0 * k).max(within.diagonal() / 1600.0),
    }
}

/// A pair whose box is moved so that a side of it passes the first point
/// where curves of its crossing cross or pass close at `inset` tolerances
/// inside the box (outside it where negative), across `towards` (a unit
/// vector along an axis of the world): the cube along the world's axes of
/// half side `half` (in the pair's units, before its scale) whose centre is
/// that far from the point the other way. (Upright, the box's sides lie
/// along the planes the figure is symmetric about; turned, they cut its
/// arms askew.)
fn at_the_side(mut pair: Pair, placed: Placed, towards: Vec3, inset: f64, half: f64) -> Pair {
    let t = pair.tol.linear;
    let point = pair.keep_off[0];
    let side = half * placed.k;
    let centre = point - towards * (side - inset * t);
    let corner = Vec3::new(side, side, side);
    pair.within = Bbox3::from_points([centre - corner, centre + corner]);
    pair.what = format!(
        "{}, moved so that its side passes the point {inset} tolerances inside it, across {towards:?}",
        pair.what
    );
    pair.keep_off_radius = (0.05 * 1e-3 * placed.k).max(pair.within.diagonal() / 1600.0);
    pair
}

/// The length of a curve of an answer, by `n` chords.
fn curve_length(curve: &MeetingCurve, n: u32) -> f64 {
    let (t0, t1) = curve.range;
    let at = |i: u32| {
        curve
            .curve
            .eval(t0 + (t1 - t0) * f64::from(i) / f64::from(n))
    };
    let mut before = at(0);
    let mut length = 0.0;
    for i in 1..=n {
        let p = at(i);
        length += before.distance(p);
        before = p;
    }
    length
}

/// The whole crossing's length inside the box (nothing left out about the
/// points), and how near it comes to each point where its curves cross or
/// pass close.
fn whole_length(pair: &Pair) -> (f64, Vec<f64>) {
    let t = pair.tol.linear;
    let mut length = 0.0;
    let mut nearest = vec![f64::INFINITY; pair.keep_off.len()];
    for sheet in &pair.sheets {
        let found = stretches(sheet, &pair.within, 8.0 * t, 1 << 16);
        length += length_inside(sheet, &pair.within, &found, 1 << 15, &|_: Vec3| true);
        let n = 1_u32 << 16;
        for i in 0..=n {
            let p = (sheet.at)(sheet.from + (sheet.to - sheet.from) * f64::from(i) / f64::from(n));
            for (j, c) in pair.keep_off.iter().enumerate() {
                nearest[j] = nearest[j].min(c.distance(p));
            }
        }
    }
    (length, nearest)
}

/// The ends of the open curves: each within two tolerances of a point where
/// curves of the crossing cross or pass close, or of a side of the box; at
/// such a point at least four tolerances inside the box, an even number.
fn ends_held(pair: &Pair, m: &Meeting) -> Result<(), String> {
    let t = pair.tol.linear;
    let mut meeting = vec![0_usize; pair.keep_off.len()];
    for (n, curve) in m.curves.iter().enumerate() {
        if curve.closed {
            continue;
        }
        for end in [curve.range.0, curve.range.1] {
            let p = curve.curve.eval(end);
            let on_box = !pair.within.contains(p, -2.0 * t);
            match pair.keep_off.iter().position(|c| c.distance(p) <= 2.0 * t) {
                Some(j) => meeting[j] += 1,
                None if on_box => {}
                None => {
                    let from = pair
                        .keep_off
                        .iter()
                        .map(|c| c.distance(p))
                        .fold(f64::INFINITY, f64::min);
                    return Err(format!(
                        "curve {n} ends at {p:?}, {} tolerances from the nearest point where curves of the crossing cross or pass close, inside the box",
                        from / t
                    ));
                }
            }
        }
    }
    for (j, c) in pair.keep_off.iter().enumerate() {
        if pair.within.contains(*c, -4.0 * t) && meeting[j] % 2 == 1 {
            return Err(format!(
                "{} ends of curves meet at {c:?}, inside the box: an odd number",
                meeting[j]
            ));
        }
    }
    Ok(())
}

/// All the curves together as long as the whole crossing in the box.
fn whole_length_held(pair: &Pair, m: &Meeting) -> Result<(), String> {
    let t = pair.tol.linear;
    let (known, nearest) = whole_length(pair);
    let got: f64 = m.curves.iter().map(|c| curve_length(c, 1 << 14)).sum();
    let slack = 32.0 * t + 1e-5 * known;
    let bend: f64 = nearest
        .iter()
        .map(|d| 4.0 * d.min(pair.within.diagonal()))
        .sum();
    if !(-slack..=slack + bend).contains(&(got - known)) {
        return Err(format!(
            "the curves are {} tolerances long, the whole crossing in the box {} (slack {} and {} for the points)",
            got / t,
            known / t,
            slack / t,
            bend / t
        ));
    }
    Ok(())
}

/// What is said of an answer held to the crossing.
const CROSSING: &str = "the crossing";
/// What an answer that is not the crossing is said to be, first.
const WRONG: &str = "WRONG";

/// Where the crossing leaves the box within the radius left out about a
/// point where its curves cross or pass close (where `held` asks nothing
/// of the curves), an end of a curve lies on the box within that radius of
/// the place: a crossing that comes to the box's side beside such a point
/// divides what lies on either side of it up to that side, and an answer
/// whose curves stop at the point leaves the two joined past it.
fn exits_held(pair: &Pair, m: &Meeting) -> Result<(), String> {
    let t = pair.tol.linear;
    let within = &pair.within;
    let ends: Vec<Vec3> = m
        .curves
        .iter()
        .filter(|c| !c.closed)
        .flat_map(|c| [c.curve.eval(c.range.0), c.curve.eval(c.range.1)])
        .filter(|p| !within.contains(*p, -2.0 * t))
        .collect();
    for sheet in &pair.sheets {
        let n = 1_u32 << 16;
        let at =
            |i: u32| (sheet.at)(sheet.from + (sheet.to - sheet.from) * f64::from(i) / f64::from(n));
        let mut before = at(0);
        for i in 1..=n {
            let p = at(i);
            if within.contains(before, 0.0) != within.contains(p, 0.0) {
                let (mut inner, mut outer) = if within.contains(before, 0.0) {
                    (before, p)
                } else {
                    (p, before)
                };
                for _ in 0..60 {
                    let mid = inner.lerp(outer, 0.5);
                    if within.contains(mid, 0.0) {
                        inner = mid;
                    } else {
                        outer = mid;
                    }
                }
                let near = pair
                    .keep_off
                    .iter()
                    .any(|c| c.distance(inner) <= pair.keep_off_radius);
                if near
                    && !ends
                        .iter()
                        .any(|e| e.distance(inner) <= pair.keep_off_radius)
                {
                    return Err(format!(
                        "the crossing leaves the box at {inner:?}, within {} tolerances of a point where its curves cross or pass close, and no curve ends on the box within that of it",
                        pair.keep_off_radius / t
                    ));
                }
            }
            before = p;
        }
    }
    Ok(())
}

/// The places where the crossing leaves the box (the last place inside,
/// found by halving between two of 65 537 places along each sheet).
fn exits(pair: &Pair) -> Vec<Vec3> {
    let within = &pair.within;
    let mut found = Vec::new();
    for sheet in &pair.sheets {
        let n = 1_u32 << 16;
        let at =
            |i: u32| (sheet.at)(sheet.from + (sheet.to - sheet.from) * f64::from(i) / f64::from(n));
        let mut before = at(0);
        for i in 1..=n {
            let p = at(i);
            if within.contains(before, 0.0) != within.contains(p, 0.0) {
                let (mut inner, mut outer) = if within.contains(before, 0.0) {
                    (before, p)
                } else {
                    (p, before)
                };
                for _ in 0..60 {
                    let mid = inner.lerp(outer, 0.5);
                    if within.contains(mid, 0.0) {
                        inner = mid;
                    } else {
                        outer = mid;
                    }
                }
                found.push(inner);
            }
            before = p;
        }
    }
    found
}

/// The converse of `exits_held` (the review of the repair of the boxes
/// whose side passes the point, 2026-11-26): about a point where curves of
/// the crossing cross or pass close that lies more than four tolerances
/// from every side of the box (inside it or out), the curves end on the
/// box as often as the crossing leaves it there. Counted within the radius
/// left out about the point and 10 tolerances, the ends of curves on the
/// box are no fewer than the places where the crossing leaves the box
/// within that radius, and no more than those within it and 20 tolerances.
/// A curve drawn to the box where the crossing does not come to it, or a
/// second one beside one that does, is an end too many.
fn exits_counted(pair: &Pair, m: &Meeting) -> Result<(), String> {
    let t = pair.tol.linear;
    let within = &pair.within;
    let ends: Vec<Vec3> = m
        .curves
        .iter()
        .filter(|c| !c.closed)
        .flat_map(|c| [c.curve.eval(c.range.0), c.curve.eval(c.range.1)])
        .filter(|p| !within.contains(*p, -2.0 * t))
        .collect();
    let leaves = exits(pair);
    let r = pair.keep_off_radius;
    for c in &pair.keep_off {
        let clear_of_sides = within.contains(*c, -4.0 * t) || !within.contains(*c, 4.0 * t);
        if !clear_of_sides {
            continue;
        }
        let count =
            |list: &[Vec3], reach: f64| list.iter().filter(|p| p.distance(*c) <= reach).count();
        let (least, most) = (count(&leaves, r), count(&leaves, r + 20.0 * t));
        let got = count(&ends, r + 10.0 * t);
        if got < least || got > most {
            return Err(format!(
                "{got} ends of curves on the box within {} tolerances of {c:?}, where the crossing leaves the box {least} times within {} and {most} within {}",
                (r + 10.0 * t) / t,
                r / t,
                (r + 20.0 * t) / t
            ));
        }
    }
    Ok(())
}

/// The four oracles, and with `strictly` the fifth (`exits_counted`).
fn judged(pair: &Pair, m: &Meeting, strictly: bool) -> Result<(), String> {
    held(pair, m)?;
    ends_held(pair, m)?;
    whole_length_held(pair, m)?;
    exits_held(pair, m)?;
    if strictly {
        exits_counted(pair, m)?;
    }
    Ok(())
}

fn verdict(pair: &Pair, met: &Result<SurfaceMeeting, GeomError>, strictly: bool) -> String {
    match met {
        Ok(SurfaceMeeting::Meets(m)) => match judged(pair, m, strictly) {
            Ok(()) => CROSSING.to_string(),
            Err(why) => format!("{WRONG}: {why}"),
        },
        // (Where the crossing does not reach into the box, nothing is
        // the crossing.)
        Ok(SurfaceMeeting::None) if whole_length(pair).0 <= 2.0 * pair.tol.linear => {
            CROSSING.to_string()
        }
        Ok(other) => format!("{WRONG}: {other:?}"),
        Err(e) => format!("refused: {e}"),
    }
}

/// A digest of an answer's bits (FNV-1a of its debug text, which writes
/// every number so that it reads back to the same bits).
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The distance from `p` to the segment from `a` to `b`.
fn to_segment(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let d = b - a;
    let l2 = d.dot(d);
    let s = if l2 > 0.0 {
        ((p - a).dot(d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + d * s).distance(p)
}

/// Each curve of an answer, for a line about it: its length, whether it
/// closes, how far each end is from the nearest point where curves of the
/// crossing cross or pass close and whether it is on the box, and how much
/// of it (further than the radius left out about those points) lies within
/// two tolerances of another curve or of a stretch of itself more than 20
/// tolerances further along it.
fn detail(pair: &Pair, m: &Meeting) -> String {
    let t = pair.tol.linear;
    let n = 600_u32;
    let runs: Vec<Vec<(Vec3, f64)>> = m
        .curves
        .iter()
        .map(|c| {
            let (t0, t1) = c.range;
            let mut s = 0.0;
            let mut before = c.curve.eval(t0);
            (0..=n)
                .map(|i| {
                    let p = c.curve.eval(t0 + (t1 - t0) * f64::from(i) / f64::from(n));
                    s += before.distance(p);
                    before = p;
                    (p, s)
                })
                .collect()
        })
        .collect();
    let from_points = |p: Vec3| {
        pair.keep_off
            .iter()
            .map(|c| c.distance(p))
            .fold(f64::INFINITY, f64::min)
    };
    let mut lines = Vec::new();
    for (a, run) in runs.iter().enumerate() {
        let mut twice = 0.0;
        for step in run.windows(2) {
            let (p, s) = step[1];
            if !pair.clear(p) {
                continue;
            }
            let mut best = f64::INFINITY;
            for (b, other) in runs.iter().enumerate() {
                for w in other.windows(2) {
                    if a == b && w[1].1 >= s - 20.0 * t && w[0].1 <= s + 20.0 * t {
                        continue;
                    }
                    // (Too far from its start to come within two
                    // tolerances anywhere along it.)
                    if p.distance(w[0].0) > w[0].0.distance(w[1].0) + 2.0 * t {
                        continue;
                    }
                    best = best.min(to_segment(p, w[0].0, w[1].0));
                }
            }
            if best <= 2.0 * t {
                twice += s - step[0].1;
            }
        }
        let curve = &m.curves[a];
        let ends = [curve.range.0, curve.range.1].map(|e| {
            let p = curve.curve.eval(e);
            format!(
                "{:.1}{}",
                from_points(p) / t,
                if pair.within.contains(p, -2.0 * t) {
                    ""
                } else {
                    " (box)"
                }
            )
        });
        lines.push(format!(
            "curve {a}: {:.1} tolerances, closed {}, ends {} and {} tolerances from the nearest point, {:.1} tolerances of it twice",
            run.last().map_or(0.0, |r| r.1) / t,
            curve.closed,
            ends[0],
            ends[1],
            twice / t
        ));
    }
    let (known, nearest) = whole_length(pair);
    let got: f64 = m.curves.iter().map(|c| curve_length(c, 1 << 14)).sum();
    lines.push(format!(
        "together {:.1} tolerances, the whole crossing in the box {:.1}; it passes the points at {:?} tolerances",
        got / t,
        known / t,
        nearest.iter().map(|d| (d / t * 10.0).round() / 10.0).collect::<Vec<_>>()
    ));
    // Where the crossing leaves the box near the points, and where curves
    // end on the box near them (tolerances from the nearest point).
    let reach = pair.keep_off_radius + 20.0 * t;
    let near = |list: Vec<Vec3>| -> Vec<f64> {
        list.into_iter()
            .map(from_points)
            .filter(|d| *d <= reach)
            .map(|d| (d / t * 10.0).round() / 10.0)
            .collect()
    };
    let box_ends: Vec<Vec3> = m
        .curves
        .iter()
        .filter(|c| !c.closed)
        .flat_map(|c| [c.curve.eval(c.range.0), c.curve.eval(c.range.1)])
        .filter(|p| !pair.within.contains(*p, -2.0 * t))
        .collect();
    lines.push(format!(
        "the crossing leaves the box near the points at {:?}, curves end on the box near them at {:?}",
        near(exits(pair)),
        near(box_ends)
    ));
    lines.join("; ")
}

/// Asks every pair of a family and writes one line for each (what was
/// answered, the digest of its bits, and the verdict), a line of detail for
/// each that is not the crossing or that `detailed` asks for, and the
/// counts. Returns the pairs that are not the crossing.
fn family(name: &str, pairs: &[Pair], detailed: &dyn Fn(&Pair) -> bool) -> Vec<String> {
    family_held(name, pairs, detailed, false)
}

/// [`family`] held to the fifth oracle too (`exits_counted`).
fn family_strictly(name: &str, pairs: &[Pair]) -> Vec<String> {
    family_held(name, pairs, &none, true)
}

fn family_held(
    name: &str,
    pairs: &[Pair],
    detailed: &dyn Fn(&Pair) -> bool,
    strictly: bool,
) -> Vec<String> {
    let mut failed = Vec::new();
    let mut counted: BTreeMap<String, usize> = BTreeMap::new();
    for pair in pairs {
        let met = meet_surfaces_numeric(&pair.a, &pair.b, &pair.within, &pair.tol);
        let said = verdict(pair, &met, strictly);
        let bits = digest(&format!("{met:?}"));
        let curves = match &met {
            Ok(SurfaceMeeting::Meets(m)) => m.curves.len(),
            _ => 0,
        };
        let short: String = said.chars().take(600).collect();
        say(&format!(
            "REVIEW-NC {name} | {} | {bits:016x} | {curves} curves | {short}",
            pair.what
        ));
        if let Ok(SurfaceMeeting::Meets(m)) = &met
            && (said != CROSSING || detailed(pair))
        {
            say(&format!(
                "REVIEW-NC-DETAIL {name} | {} | {}",
                pair.what,
                detail(pair, m)
            ));
        }
        let kind = if said == CROSSING {
            CROSSING
        } else if said.starts_with("refused") {
            "refused"
        } else {
            WRONG
        };
        *counted.entry(kind.to_string()).or_insert(0) += 1;
        if said != CROSSING {
            failed.push(format!("{}: {said}", pair.what));
        }
    }
    for (kind, n) in &counted {
        say(&format!(
            "REVIEW-NC-COUNT {name}: {n} of {}: {kind}",
            pairs.len()
        ));
    }
    failed
}

/// The gaps the opened figures are asked at, in radii: a millionth to a
/// hundredth, either way.
const GAPS: [f64; 14] = [
    -1e-2, -1e-3, -3e-4, -1e-4, -3e-5, -1e-5, -1e-6, 1e-6, 1e-5, 3e-5, 1e-4, 3e-4, 1e-3, 1e-2,
];

/// The sizes a family is asked in: every other of the eighteen the review
/// of the fold asks (1.5 to 380 radii), all eighteen at `η = ±1e-4`.
fn sizes_at(eta: f64) -> Vec<f64> {
    let all = sizes(1.5, 18);
    if (eta.abs() - 1e-4).abs() < 1e-12 {
        all
    } else {
        all.into_iter().step_by(2).collect()
    }
}

/// Fewer sizes: 1.5 to 192 radii.
fn few_sizes() -> Vec<f64> {
    vec![1.5, 3.0, 6.0, 12.0, 48.0, 192.0]
}

/// Detail for the opened figures at `η = ±1e-4`.
fn at_a_ten_thousandth(pair: &Pair) -> bool {
    pair.what.contains("eta 0.0001,") || pair.what.contains("eta -0.0001,")
}

fn none(_: &Pair) -> bool {
    false
}

fn fails(name: &str, failed: &[String]) {
    say(&format!("REVIEW-NC-FAILED {name}: {}", failed.len()));
    assert!(
        failed.is_empty(),
        "{name}: {} answers are not the crossing:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// Viviani's figure opened by every gap (14, a millionth to a hundredth,
/// either way), in cubes of 1.5 to 380 ball radii about where it crossed:
/// 1 152 pairs, 110 not the crossing and 6 refused on `main` (at
/// `η = ±1e-4` and `±3e-4`), all the crossing with the branch.
#[test]
fn vivianis_figure_opened_by_every_gap_is_its_crossing() {
    let pairs = every(|placed, how, pairs| {
        for eta in GAPS {
            for size in sizes_at(eta) {
                pairs.push(ball_and_tube_opened(placed, eta, size, how));
            }
        }
    });
    fails(
        "ball and tube",
        &family("ball and tube", &pairs, &at_a_ten_thousandth),
    );
}

/// Steinmetz's ellipses opened by every gap, in cubes of 1.5 to 380 radii
/// about a point where they crossed: 1 152 pairs, 168 not the crossing on
/// `main` (at `η = ±1e-4` and `±3e-4`), all the crossing with the branch.
#[test]
fn steinmetz_ellipses_opened_by_every_gap_are_their_crossing() {
    let pairs = every(|placed, how, pairs| {
        for eta in GAPS {
            for size in sizes_at(eta) {
                pairs.push(tubes_opened(placed, eta, size, how));
            }
        }
    });
    fails(
        "two tubes",
        &family("two tubes", &pairs, &at_a_ten_thousandth),
    );
}

/// Two cylinders whose axes cross at 30, 45 and 60 degrees, the second a
/// little wider or narrower (and of one radius), at two scales: 432
/// pairs, 108 not the crossing and 8 refused on `main`, all the crossing
/// with the branch.
#[test]
fn tubes_crossing_at_an_angle_opened_are_their_crossing() {
    let mut pairs = Vec::new();
    for scale in [1.0, 1e6] {
        for (how, placed) in placings(scale) {
            for gamma in [PI / 6.0, PI / 4.0, PI / 3.0] {
                for eta in [-1e-4, -1e-5, 0.0, 1e-5, 1e-4, 3e-4] {
                    for size in few_sizes() {
                        pairs.push(tubes_at_opened(placed, gamma, eta, size, how));
                    }
                }
            }
        }
    }
    fails(
        "tubes at an angle",
        &family("tubes at an angle", &pairs, &none),
    );
}

/// The tori and planes of the next two tests, the ones `owed` asks for or
/// the others.
fn tori_and_planes(owed: bool) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for scale in [1.0, 1e3] {
        for (how, placed) in placings(scale) {
            for q in [1.01, 1.05, 1.2, 2.0, 4.0] {
                for seam in [Vec3::X, Vec3::Y] {
                    for eta in [-3e-4, -1e-4, -1e-5, 0.0, 1e-5, 1e-4, 3e-4] {
                        if owed != (q < 1.02 && eta > 2e-4) {
                            continue;
                        }
                        for size in [1.5, 6.0, 24.0, 96.0] {
                            pairs.push(torus_and_plane(placed, q, seam, eta, size, how));
                        }
                    }
                }
            }
        }
    }
    pairs
}

/// A plane beside the inner equator of a torus: tori whose arms there are
/// 11, 25, 48, 90 and 120 degrees apart (`R/ρ` 1.01, 1.05, 1.2, 2 and 4),
/// the seam through the point and a quarter turn away, the plane a
/// little nearer the axis or further (and touching), at two scales: 1 088
/// pairs, 40 not the crossing on `main` and 32 refused (Check 45711338836),
/// all the crossing with the branch (Check 45711343773).
#[test]
fn a_plane_beside_the_inner_equator_of_a_torus_is_its_crossing() {
    let pairs = tori_and_planes(false);
    fails("torus and plane", &family("torus and plane", &pairs, &none));
}

/// The plane three ten-thousandths of a radius into the tube of the torus
/// whose hole is a hundredth of its tube (arms 11 degrees apart): in the
/// smallest cubes (1.5 radii) a curve drawn straight into the point is 12
/// tolerances off the crossing 12 tolerances from it, where `held` allows
/// 8.5; the same bits on `main` (Check 45711338836) as with the branch
/// (Check 45711343773), 8 of 32. Within the zone of the point the march
/// draws its curves straight, and the surfaces are within the tolerance
/// of each other there: whether the oracle or the zone is to give is not
/// this review's to settle.
#[test]
#[ignore = "owed, as on main (Check 45711338836, the same bits as Check 45711343773): a curve drawn straight into the point 12 tolerances off the crossing of a thin-holed torus and a plane; run with --ignored"]
fn a_plane_into_a_thin_holed_torus_at_its_inner_equator_is_its_crossing() {
    let pairs = tori_and_planes(true);
    fails(
        "torus and plane, owed",
        &family("torus and plane, owed", &pairs, &none),
    );
}

/// A cone and a cylinder that both nearly touch one ball (cones of 20, 35
/// and 50 degrees; the cylinder a little narrower or wider, and touching),
/// at two scales: 360 pairs, 46 not the crossing on `main`, all the
/// crossing with the branch.
#[test]
fn a_cone_and_a_cylinder_round_one_ball_are_their_crossing() {
    let mut pairs = Vec::new();
    for scale in [1.0, 1e6] {
        for (how, placed) in placings(scale) {
            for alpha in [20.0_f64, 35.0, 50.0] {
                for eta in [-1e-4, -1e-5, 0.0, 1e-5, 1e-4] {
                    for size in few_sizes() {
                        pairs.push(cone_and_cylinder(
                            placed,
                            alpha.to_radians(),
                            eta,
                            size,
                            how,
                        ));
                    }
                }
            }
        }
    }
    fails(
        "cone and cylinder",
        &family("cone and cylinder", &pairs, &none),
    );
}

/// The opened figures in cubes of 768 to 3 072 radii, where the zone of a
/// point (at least 1.25 times a hundredth of the box's diagonal over 32)
/// holds much of the figure or all of it, and in the larger two the two
/// points of Steinmetz's are each in the other's zone.
///
/// Viviani's in all three, Steinmetz's in the smaller two: 40 pairs, the
/// crossing on `main` and with the branch, the same bits.
#[test]
fn opened_figures_in_huge_boxes_are_their_crossing() {
    let pairs = huge(false);
    fails("huge boxes", &family("huge boxes", &pairs, &none));
}

/// The pairs of the huge boxes, the ones `owed` asks for or the others.
fn huge(owed: bool) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for scale in [1.0, 1e3] {
        for (how, placed) in placings(scale) {
            for eta in [-1e-4, 1e-4] {
                for size in [768.0, 1536.0, 3072.0] {
                    if owed {
                        pairs.push(torus_and_plane(placed, 2.0, Vec3::Y, eta, size, how));
                        if size > 2000.0 {
                            pairs.push(tubes_opened(placed, eta, size, how));
                        }
                    } else {
                        pairs.push(ball_and_tube_opened(placed, eta, size, how));
                        if size < 2000.0 {
                            pairs.push(tubes_opened(placed, eta, size, how));
                        }
                    }
                }
            }
        }
    }
    pairs
}

/// The torus and the plane in cubes of 768 to 3 072 tube radii are refused
/// (*"an intersection curve could not be fitted within the tolerance"*,
/// 24 of 24), and Steinmetz's turned in the cube of 3 072 radii is answered
/// as no meeting at all (4 of 8): on `main` (Check 45711338836) as with the
/// branch (Check 45711343773), the same bits. Neither is of the zone: no
/// point where the curves cross is read where nothing is found.
#[test]
#[ignore = "owed, as on main (Check 45711338836, the same bits as Check 45711343773): a torus and a plane refused, and Steinmetz's turned answered as no meeting, in boxes thousands of times their size; run with --ignored"]
fn a_torus_and_a_plane_and_steinmetz_in_huge_boxes_are_their_crossing() {
    let pairs = huge(true);
    fails(
        "huge boxes, owed",
        &family("huge boxes, owed", &pairs, &none),
    );
}

/// The opened figures, turned, in cubes of three radii along the world's
/// axes whose side passes the point 40 tolerances outside the box to 120
/// inside it, across each of the six directions along those axes: 168
/// pairs, 83 not the crossing and 4 refused on `main` (Check 45713376999),
/// 16 and 2 with the near-crossing repair (Check 45713380970), each of them
/// not the crossing or refused on `main` too; 4 that were the crossing on
/// `main` moved their bits and are the crossing still. All 168 the
/// crossing since the repair of the boxes whose side passes the point
/// (2026-11-26; docs/design/booleans.md › *A box whose side passes a
/// crossing point the surfaces are a little apart at*).
#[test]
fn opened_figures_whose_box_cuts_the_zone_are_their_crossing() {
    fails(
        "box through the zone",
        &family("box through the zone", &through_the_zone(1), &none),
    );
}

/// The same upright, the box's sides along the planes the figures are
/// symmetric about: 168 pairs, 78 not the crossing or refused on
/// `main` (Check 45711338836) and 23 with the near-crossing repair (Check
/// 45711343773), each of them not the crossing on `main` either: two
/// curves over one stretch, a stub of an arm missing, a curve that leaves
/// the box, a refusal; 26 by the four oracles (the exits found three more,
/// each with the two arms towards the side missing where the crossing
/// leaves the box beside the point). All 168 the crossing since the repair
/// of the boxes whose side passes the point (2026-11-26).
#[test]
fn upright_opened_figures_whose_box_cuts_the_zone_are_their_crossing() {
    fails(
        "box through the zone, upright",
        &family("box through the zone, upright", &through_the_zone(0), &none),
    );
}

/// The pairs whose box's side passes the point, placed the `which` way of
/// [`placings`] (0 upright, 1 turned).
fn through_the_zone(which: usize) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for (how, placed) in placings(1.0).into_iter().skip(which).take(1) {
        for eta in [-1e-4, 1e-4] {
            for towards in [
                Vec3::X,
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::Y,
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::Z,
                Vec3::new(0.0, 0.0, -1.0),
            ] {
                for inset in [-40.0, -10.0, 0.0, 10.0, 40.0, 80.0, 120.0] {
                    let half = 3e-3;
                    pairs.push(at_the_side(
                        ball_and_tube_opened(placed, eta, 3.0, how),
                        placed,
                        towards,
                        inset,
                        half,
                    ));
                    pairs.push(at_the_side(
                        tubes_opened(placed, eta, 3.0, how),
                        placed,
                        towards,
                        inset,
                        half,
                    ));
                }
            }
        }
    }
    pairs
}

/// The six directions along the world's axes.
const SIDES: [Vec3; 6] = [
    Vec3::X,
    Vec3::new(-1.0, 0.0, 0.0),
    Vec3::Y,
    Vec3::new(0.0, -1.0, 0.0),
    Vec3::Z,
    Vec3::new(0.0, 0.0, -1.0),
];

/// The boxes through the zone of the two tests above, held to the fifth
/// oracle too (`exits_counted`; the review of the repair of the boxes whose
/// side passes the point, 2026-11-26): 44 of 336 not the crossing or
/// refused on the base d835d891 (Check 45763928388), 17 with the repair
/// (Check 45763931681), each of them wrong on the base too. Each of the 17:
/// two arms drawn straight from the point to the box's side where the
/// crossing turns round the point short of that side and leaves the box
/// nowhere within 70 tolerances of it (upright Viviani's opened outwards,
/// the side 10 tolerances beyond the point across y: the branches turn 20
/// tolerances to either side, and the answer has two curves of 13.6 and
/// 13.7 tolerances from the point to the side). Not the stubs (with them
/// switched off the same 17 have the same bits, probe Check 45774968238):
/// the box cut, which keeps the span as drawn where Newton's place on
/// `f = 0` is not on the side.
#[test]
#[ignore = "owed, as on the base (Check 45763928388; 17 of 336 with the repair, Check 45763931681, each wrong on the base too): arms drawn to the box's side where the crossing turns short of it; run with --ignored"]
fn boxes_through_the_zone_have_ends_on_the_box_only_where_the_crossing_leaves_it() {
    let mut pairs = through_the_zone(0);
    pairs.extend(through_the_zone(1));
    fails(
        "box through the zone, strictly",
        &family_strictly("box through the zone, strictly", &pairs),
    );
}

/// The opened figures (one and three ten-thousandths, either way) in cubes
/// of three radii whose side passes the point 25 tolerances outside the box
/// to 30 inside it, between the insets of the tests above, near where the
/// crossing turns round the point (14 to 35 tolerances from it), upright
/// and turned, held to the five oracles (the review of the repair of the
/// boxes whose side passes the point, 2026-11-26): 156 of 768 not the
/// crossing or refused on the base d835d891 (Check 45763928388), 93 with
/// the repair (Check 45763931681), each of them wrong on the base too; 79
/// of the 93 by the fifth oracle alone.
#[test]
#[ignore = "owed, as on the base (Check 45763928388; 93 of 768 with the repair, Check 45763931681, each wrong on the base too): opened figures whose box's side passes near where the crossing turns round the point; run with --ignored"]
fn opened_figures_through_the_zone_at_fine_insets_are_their_crossing() {
    fails(
        "fine insets",
        &family_strictly("fine insets", &fine_insets()),
    );
}

/// The pairs of the test above.
fn fine_insets() -> Vec<Pair> {
    let mut pairs = Vec::new();
    for (how, placed) in placings(1.0) {
        for eta in [-3e-4, -1e-4, 1e-4, 3e-4] {
            for towards in SIDES {
                for inset in [-25.0, -15.0, -5.0, 5.0, 15.0, 20.0, 25.0, 30.0] {
                    pairs.push(at_the_side(
                        ball_and_tube_opened(placed, eta, 3.0, how),
                        placed,
                        towards,
                        inset,
                        3e-3,
                    ));
                    pairs.push(at_the_side(
                        tubes_opened(placed, eta, 3.0, how),
                        placed,
                        towards,
                        inset,
                        3e-3,
                    ));
                }
            }
        }
    }
    pairs
}

/// Tori (arms 25 and 90 degrees apart at the inner equator; the seam a
/// quarter turn from the point) and planes, and a cone of 35 degrees and a
/// cylinder round one ball, in cubes of three radii whose side passes the
/// point 10 tolerances outside to 40 inside, upright and turned, held to
/// the five oracles: 79 of 360 not the crossing or refused on the base
/// (Check 45763928388), 20 with the repair (Check 45763931681), each of
/// them wrong on the base too; 15 of the 20 by the fifth oracle alone.
#[test]
#[ignore = "owed, as on the base (Check 45763928388; 20 of 360 with the repair, Check 45763931681, each wrong on the base too): tori and a cone whose box's side passes the point; run with --ignored"]
fn tori_and_a_cone_through_the_zone_are_their_crossing() {
    fails(
        "others through the zone",
        &family_strictly("others through the zone", &others_through_the_zone()),
    );
}

/// The pairs of the test above.
fn others_through_the_zone() -> Vec<Pair> {
    let mut pairs = Vec::new();
    for (how, placed) in placings(1.0) {
        for eta in [-1e-4, 1e-4] {
            for towards in SIDES {
                for inset in [-10.0, 0.0, 10.0, 20.0, 40.0] {
                    for q in [1.05, 2.0] {
                        pairs.push(at_the_side(
                            torus_and_plane(placed, q, Vec3::Y, eta, 3.0, how),
                            placed,
                            towards,
                            inset,
                            3e-3,
                        ));
                    }
                    pairs.push(at_the_side(
                        cone_and_cylinder(placed, 35.0_f64.to_radians(), eta, 3.0, how),
                        placed,
                        towards,
                        inset,
                        3e-3,
                    ));
                }
            }
        }
    }
    pairs
}
