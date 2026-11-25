//! The review of `check/march-fold` (2026-11-24): curves of a crossing
//! that turn tightly and run one way, on which no fold may be read
//! (docs/design/booleans.md › *A step through a point where two curves of
//! a crossing cross each other: traced and mended*).
//!
//! A fold is two spans in a row of what the fit accepted, each longer than
//! four tolerances, whose chords turn by more than 120 degrees. A curve
//! followed in one pass turns between two of its points by no more than
//! its tangent turns between them, and the march holds the turn between
//! the ends of a step to 0.2 radians, not the turn inside it. So these
//! pairs are curves that turn by more than 120 degrees within a few to a
//! few hundred tolerances, asked in boxes from a few times that size to
//! thousands of times it, where the march's first steps are many times the
//! turn:
//!
//! - the tip of a long thin ellipse (a plane nearly along a thin cylinder);
//! - two cylinders whose axes are skew at a small angle and that overlap
//!   by a little: a thin slanted loop with two tight tips, the surfaces
//!   nearly tangent along it (a hairpin where two surfaces almost touch);
//! - a ball and a cylinder inside it of nearly half its radius, nearly
//!   touching it or poking out of it a little (Viviani's figure of eight
//!   opened at its crossing point: two loops, or one curve with two tight
//!   turns beside that point);
//! - two cylinders at right angles of nearly one radius (Steinmetz's two
//!   ellipses opened at their crossing points: two loops that pass close);
//! - the small circle where a plane cuts a cap off a ball (a curve near a
//!   tangency), in boxes far larger than it;
//!
//! and, with crossing points, where a fold may be read and must be
//! mended: two cylinders of one radius whose axes cross at 30, 45, 60 and
//! 90 degrees, in boxes about a crossing point.
//!
//! Each at four scales (1e-3 to 1e6), upright and turned. Each is asked of
//! the marcher alone as the march is first made and with branches taken up
//! again (`meet_surfaces_numeric_once`, `meet_surfaces_numeric`); the
//! second is held to the crossing as known in closed form (every curve on
//! both surfaces and in the box, every point of it on the crossing, the
//! curves together as long as the crossing in the box, every place of the
//! crossing in the box on a curve; where curves of the crossing cross or
//! pass close, a ball about the place is left out of the lengths and of
//! the places asked), and where the first answers the two are the same bit
//! for bit. No fold was read on any pair whose curves run one way.
//!
//! The opened figures, held to the crossing, were not all the crossing:
//! where the surfaces are a tenth of a tolerance apart at the place the
//! curves crossed, the march reads a crossing point there and in some
//! boxes gave a stretch twice, with no fold read (a wrong answer the fold
//! did not touch; repaired since, 2026-11-25:
//! `near_crossings_read_as_crossing_points_give_no_stretch_twice`).
//! Their families hold the fold and the bits only.

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

use common::{cylinder, distance, normal_at, plane, sphere};
use craft3d_geom::intersect::{
    Meeting, SurfaceMeeting, meet_surfaces, meet_surfaces_numeric, meet_surfaces_numeric_once,
};
use craft3d_geom::{GeomError, Surface};
use craft3d_math::{Bbox3, Frame, Portable, Tolerance, Vec3};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4, FRAC_PI_6, PI};

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

/// What the march as first made said where a branch it fitted ran back
/// over itself.
const FOLDED: &str = "a fold";
/// What is said of an answer held to the crossing.
const CROSSING: &str = "the crossing";
/// What an answer that is not the crossing is said to be, first.
const WRONG: &str = "WRONG";
/// What is said where the surfaces were found within the tolerance of
/// each other throughout the box (a plane along the tip of a thin
/// cylinder in a small box): not a crossing, and not held to one.
const COINCIDENT: &str = "coincident";

/// How a pair was answered: as the march is first made, with branches
/// taken up again, and whether the two are the same bit for bit where
/// both answer.
struct Asked {
    first: String,
    again: String,
    same: bool,
}

fn asked(pair: &Pair) -> Asked {
    let once = meet_surfaces_numeric_once(&pair.a, &pair.b, &pair.within, &pair.tol);
    let taken_up = meet_surfaces_numeric(&pair.a, &pair.b, &pair.within, &pair.tol);
    let first = match &once {
        Ok(SurfaceMeeting::Meets(_)) => "answered".to_string(),
        Ok(SurfaceMeeting::Coincident { .. }) => COINCIDENT.to_string(),
        Ok(other) => format!("{other:?}"),
        Err(e) if e.to_string().contains("run back over themselves") => FOLDED.to_string(),
        Err(e) => format!("refused: {e}"),
    };
    let again = match &taken_up {
        Ok(SurfaceMeeting::Meets(m)) => match held(pair, m) {
            Ok(()) => CROSSING.to_string(),
            Err(why) => format!("{WRONG}: {why}"),
        },
        Ok(SurfaceMeeting::Coincident { .. }) => COINCIDENT.to_string(),
        Ok(other) => format!("{WRONG}: {other:?}"),
        Err(e) => format!("refused: {e}"),
    };
    // (Where the march as first made answers, the march taken up again
    // makes the same first march and has nothing to take up.)
    let same = match (&once, &taken_up) {
        (Ok(a), Ok(b)) => format!("{a:?}") == format!("{b:?}"),
        (Ok(_), Err(_)) => false,
        (Err(_), _) => true,
    };
    Asked { first, again, same }
}

/// Asks every pair of a family; writes out each that is not answered the
/// same both ways by the crossing, and how many were answered how. Fails
/// where a fold is read on a family whose curves run one way
/// (`one_way`), where the march as first made answers otherwise than
/// taken up again, and, where the answers are held (`hold`), where the
/// answer taken up again is not the crossing.
fn run(family: &str, pairs: &[Pair], one_way: bool, hold: bool) {
    let mut counted: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut failed: Vec<String> = Vec::new();
    for pair in pairs {
        let got = asked(pair);
        let fine = got.first == "answered" && got.again == CROSSING && got.same;
        if !fine {
            say(&format!(
                "REVIEW-FOLD {family}: first {}; again {}; same {}: {}",
                got.first, got.again, got.same, pair.what
            ));
        }
        if one_way && got.first == FOLDED {
            failed.push(format!(
                "a fold read on a crossing that runs one way: {}",
                pair.what
            ));
        }
        if !got.same {
            failed.push(format!(
                "answered as first made, and otherwise taken up again: {}",
                pair.what
            ));
        }
        if hold && got.again.starts_with(WRONG) {
            failed.push(format!("{}: {}", got.again, pair.what));
        }
        let again = if got.again.starts_with(WRONG) {
            WRONG.to_string()
        } else {
            got.again
        };
        *counted.entry((got.first, again)).or_insert(0) += 1;
    }
    for ((first, again), n) in &counted {
        say(&format!(
            "REVIEW-FOLD {family}: {n} of {}: first {first}; again {again}",
            pairs.len()
        ));
    }
    assert!(
        failed.is_empty(),
        "{family}: {} failed\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// A plane nearly along a thin cylinder: the tip of a long thin ellipse.
/// The cylinder of radius `r = ρ A` along z through `(−r, 0, 0)`, and the
/// plane through the origin whose normal leans from x towards −z by the
/// angle `ε` of sine `1/A`: they cross in an ellipse of semi-axes `r` and
/// `r A` whose tip at the origin turns on a radius `ρ`, the surfaces at
/// the angle `ε` there. About the cylinder's axis at the angle `θ` the
/// crossing is `(−2r sin²(θ/2), r sin θ, −2r sin²(θ/2) cot ε)`. The cube
/// of half side `half` about `off · half` from the tip.
fn ellipse_tip(placed: Placed, rho: f64, aspect: f64, half: f64, off: Vec3, how: &str) -> Pair {
    let k = placed.k;
    let r = rho * aspect;
    let sine = 1.0 / aspect;
    let cosine = (1.0 - sine * sine).sqrt();
    let cot = cosine / sine;
    Pair {
        what: format!(
            "ellipse tip at {k}, {how}: tip radius {rho}, aspect {aspect}, cube of half side {half} about {off:?} of it"
        ),
        a: cylinder(
            placed.frame_of(Vec3::new(-r, 0.0, 0.0), Vec3::Z, Vec3::X),
            r * k,
        ),
        b: plane(placed.frame_of(Vec3::ZERO, Vec3::new(cosine, 0.0, -sine), Vec3::Y)),
        within: placed.cube(off * half, half),
        tol: tolerance(k),
        sheets: vec![Sheet {
            at: Box::new(move |theta: f64| {
                let h = (0.5 * theta).psin();
                let x = -2.0 * r * h * h;
                placed.point(Vec3::new(x, r * theta.psin(), x * cot))
            }),
            from: -PI,
            to: PI,
        }],
        keep_off: Vec::new(),
        keep_off_radius: 0.0,
    }
}

/// Two cylinders of radius `r` (1e-3) whose axes are skew at the small
/// angle `β`, overlapping by `δ = overlap · r` where they come nearest:
/// the first along z through the origin, the second through
/// `(2r − δ, 0, 0)` along `(0, sin β, cos β)`. They cross in one thin
/// slanted loop about `(r, 0, 0)`, the surfaces nearly tangent along it,
/// with two tight tips: about the first's axis at the angle `θ`, within
/// `θ0` (`sin²(θ0/2) = δ/2r`), its two sides are at
/// `z = (r sin θ cos β ± √D) / sin β`,
/// `D = (δ − 2r sin²(θ/2))(2r − δ + 2r sin²(θ/2))`; `θ = θ0 sin s` along
/// them. The cube of half side `size` times about half the loop's length,
/// about its middle.
fn skew_tubes(placed: Placed, overlap: f64, beta: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let r = 1e-3;
    let delta = overlap * r;
    let c = 2.0 * r - delta;
    let theta0 = 2.0 * (delta / (2.0 * r)).sqrt().pasin();
    let (sb, cb) = (beta.psin(), beta.pcos());
    let half = size * (2.0 * r * delta).sqrt() / sb;
    let side = move |sign: f64| -> Sheet {
        Sheet {
            at: Box::new(move |s: f64| {
                let theta = theta0 * s.psin();
                let h = (0.5 * theta).psin();
                let h2 = h * h;
                let d = ((delta - 2.0 * r * h2) * (2.0 * r - delta + 2.0 * r * h2)).max(0.0);
                let z = (r * theta.psin() * cb + sign * d.sqrt()) / sb;
                placed.point(Vec3::new(r * theta.pcos(), r * theta.psin(), z))
            }),
            from: -FRAC_PI_2,
            to: FRAC_PI_2,
        }
    };
    Pair {
        what: format!(
            "skew tubes at {k}, {how}: overlap {overlap} radii, axes at {beta} rad, cube of half side {half}"
        ),
        a: cylinder(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), r * k),
        b: cylinder(
            placed.frame_of(Vec3::new(c, 0.0, 0.0), Vec3::new(0.0, sb, cb), Vec3::X),
            r * k,
        ),
        within: placed.cube(Vec3::new(r, 0.0, 0.0) + OFF * half, half),
        tol: tolerance(k),
        sheets: vec![side(1.0), side(-1.0)],
        keep_off: Vec::new(),
        keep_off_radius: 0.0,
    }
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

/// A ball of radius `R = ρ / sin α` about the origin and the plane
/// `z = R cos α`: a circle of radius `ρ`, the surfaces at the angle `α`
/// all along it. The cube of half side `size · ρ` about its centre.
fn cap(placed: Placed, alpha: f64, rho: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let big = rho / alpha.psin();
    let height = big * alpha.pcos();
    let half = size * rho;
    Pair {
        what: format!(
            "a cap off a ball at {k}, {how}: circle of radius {rho} at {alpha} rad, cube of half side {half}"
        ),
        a: sphere(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), big * k),
        b: plane(placed.frame_of(Vec3::new(0.0, 0.0, height), Vec3::Z, Vec3::X)),
        within: placed.cube(Vec3::new(0.0, 0.0, height) + OFF * half, half),
        tol: tolerance(k),
        sheets: vec![Sheet {
            at: Box::new(move |s: f64| {
                placed.point(Vec3::new(rho * s.pcos(), rho * s.psin(), height))
            }),
            from: -PI,
            to: PI,
        }],
        keep_off: Vec::new(),
        keep_off_radius: 0.0,
    }
}

/// Two cylinders of radius `r` (1e-3) whose axes cross at the origin at
/// the angle `γ`: the first along z, the second along `(sin γ, 0, cos γ)`.
/// They cross in two ellipses, `z = r cos θ sin γ / (1 − cos γ)` and
/// `z = −r cos θ sin γ / (1 + cos γ)` at the first's angle `θ`, which cross
/// each other where the cylinders touch, `(0, ±r, 0)`. The cube of half
/// side `size · r` about `(0, r, 0)`.
fn tubes_at(placed: Placed, gamma: f64, size: f64, how: &str) -> Pair {
    let k = placed.k;
    let r = 1e-3;
    let (sg, cg) = (gamma.psin(), gamma.pcos());
    let ellipse = move |lean: f64| -> Sheet {
        Sheet {
            at: Box::new(move |s: f64| {
                let c = s.pcos();
                placed.point(Vec3::new(r * c, r * s.psin(), r * c * lean))
            }),
            from: -PI,
            to: PI,
        }
    };
    let half = size * r;
    let within = placed.cube(Vec3::new(0.0, r, 0.0) + OFF * half, half);
    Pair {
        what: format!("two tubes at {gamma} rad at {k}, {how}: cube of half side {half}"),
        a: cylinder(placed.frame_of(Vec3::ZERO, Vec3::Z, Vec3::X), r * k),
        b: cylinder(
            placed.frame_of(Vec3::ZERO, Vec3::new(sg, 0.0, cg), Vec3::Y),
            r * k,
        ),
        within,
        tol: tolerance(k),
        sheets: vec![ellipse(sg / (1.0 - cg)), ellipse(-sg / (1.0 + cg))],
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

/// The tip of a long thin ellipse, turning on 8, 60 and 500 tolerances,
/// of aspect 20 and 400, in cubes of 6 to 4 300 tip radii, about the tip
/// and off it: no fold is read, and every answer is the crossing (or, in
/// the smallest cubes about the thinnest tip, the surfaces within the
/// tolerance of each other throughout).
#[test]
fn the_tip_of_a_long_thin_ellipse_reads_no_fold() {
    let pairs = every(|placed, how, pairs| {
        for rho in [8e-6, 6e-5, 5e-4] {
            for aspect in [20.0, 400.0] {
                for size in sizes(6.0, 20) {
                    for off in [Vec3::ZERO, Vec3::new(-0.21, 0.13, -0.37)] {
                        pairs.push(ellipse_tip(placed, rho, aspect, size * rho, off, how));
                    }
                }
            }
        }
    });
    run("ellipse tip", &pairs, true, true);
}

/// Two cylinders skew at a small angle that overlap a little: a thin
/// slanted loop whose tips turn on a few to a few tens of tolerances, the
/// surfaces nearly tangent along it, in cubes from a part of it to 38
/// times it: no fold is read, and every answer is the crossing.
#[test]
fn a_hairpin_where_two_skew_tubes_almost_touch_reads_no_fold() {
    let pairs = every(|placed, how, pairs| {
        for (overlap, beta) in [(1e-2, 0.2), (1e-2, 0.5), (0.1, 0.1), (0.1, 0.5)] {
            for size in sizes(0.3, 14) {
                pairs.push(skew_tubes(placed, overlap, beta, size, how));
            }
        }
    });
    run("skew tubes", &pairs, true, true);
}

/// Viviani's figure of eight opened at its crossing point (the cylinder
/// a hundredth and a ten-thousandth narrower, a millionth, a
/// ten-thousandth and a hundredth wider).
fn vivianis_opened() -> Vec<Pair> {
    every(|placed, how, pairs| {
        for eta in [-1e-2, -1e-4, 1e-6, 1e-4, 1e-2] {
            for size in sizes(1.5, 18) {
                pairs.push(ball_and_tube_opened(placed, eta, size, how));
            }
        }
    })
}

/// Steinmetz's ellipses opened at their crossing points (the second
/// cylinder a millionth, a ten-thousandth and a hundredth wider, a
/// ten-thousandth and a hundredth narrower).
fn steinmetz_opened() -> Vec<Pair> {
    every(|placed, how, pairs| {
        for eta in [1e-6, 1e-4, 1e-2, -1e-4, -1e-2] {
            for size in sizes(1.5, 18) {
                pairs.push(tubes_opened(placed, eta, size, how));
            }
        }
    })
}

/// Viviani's figure opened, in cubes of 1.5 to 380 ball radii about where
/// it crossed: no fold is read, and the march as first made answers as
/// the march taken up again.
#[test]
fn vivianis_figure_opened_at_its_crossing_reads_no_fold() {
    run("ball and tube opened", &vivianis_opened(), true, false);
}

/// Steinmetz's ellipses opened, in cubes of 1.5 to 380 radii about where
/// they crossed: no fold is read, and the march as first made answers as
/// the march taken up again.
#[test]
fn steinmetz_ellipses_opened_at_their_crossings_read_no_fold() {
    run("two tubes opened", &steinmetz_opened(), true, false);
}

/// **A wrong answer this review found, not of the fold; repaired since
/// (2026-11-25).** Where Viviani's and Steinmetz's are opened by a
/// ten-thousandth of a radius, the surfaces are a tenth of a tolerance
/// apart where the curves crossed, and the march reads a point where they
/// cross there, a little apart; in some boxes it gave a stretch of the
/// crossing twice. Found by this review (Check 45288306141: in the two
/// families above, 54 and 128 pairs of 720, every one at `η = ±1e-4`, at
/// every scale, upright and turned), with the march as first made giving
/// the same answer bit for bit and no fold read. Three of them, upright
/// at scale 1, as they were:
///
/// - the two tubes, `η = 1e-4`, in the cube of 2.1 radii: two curves that
///   are the same half of a loop, from one of the two points to the other;
/// - the same in the cube of 4.2 radii: two curves that each go once round
///   a loop, from one point back to it, over the halves of the loops that
///   are given besides;
/// - the ball and tube, `η = −1e-4`, in the cube of 4.2 ball radii: one
///   curve that goes twice round a loop, from the point back to it; and
///   asked as a boolean asks it (`meet_surfaces`, which has no closed form
///   for a ball and a cylinder), the same. (Two cylinders whose radii are
///   within the tolerance of each other meet in closed form there.)
///
/// Each came from a seed inside the zone of such a point (between half the
/// radius its arms are seeded at and the radius within which a branch is
/// drawn straight to it): one beside an arm a branch already came into the
/// point along, a few tolerances off what was drawn and so not seen to be
/// on it, traced again; one so near the point that its run towards it
/// could not see it straight ahead and went round it onto the next arm,
/// which the trace's other run, or another branch, had come into the point
/// along (docs/design/booleans.md › *A stretch given twice beside a
/// crossing point the surfaces are a little apart at*).
#[test]
fn near_crossings_read_as_crossing_points_give_no_stretch_twice() {
    let placed = placings(1.0)[0].1;
    let size = sizes(1.5, 18);
    let mut failed = Vec::new();
    let mut held_by = |by: &str, pair: &Pair, met: Result<SurfaceMeeting, GeomError>| {
        let why = match met {
            Ok(SurfaceMeeting::Meets(m)) => held(pair, &m).err(),
            other => Some(format!("{other:?}")),
        };
        if let Some(why) = why {
            failed.push(format!("{} ({by}): {why}", pair.what));
        }
    };
    for pair in [
        tubes_opened(placed, 1e-4, size[1], "upright"),
        tubes_opened(placed, 1e-4, size[3], "upright"),
        ball_and_tube_opened(placed, -1e-4, size[3], "upright"),
    ] {
        let met = meet_surfaces_numeric(&pair.a, &pair.b, &pair.within, &pair.tol);
        held_by("the marcher", &pair, met);
    }
    let pair = ball_and_tube_opened(placed, -1e-4, size[3], "upright");
    let met = meet_surfaces(&pair.a, &pair.b, &pair.within, &pair.tol);
    held_by("as a boolean asks it", &pair, met);
    for line in &failed {
        say(&format!("REVIEW-TWICE {line}"));
    }
    assert!(
        failed.is_empty(),
        "{} answers are not the crossing:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// An end of a curve that is neither at a place where curves of the
/// crossing cross or pass close (within two tolerances of it: the march
/// reads a crossing point there, the surfaces a tenth of a tolerance apart
/// or less) nor on the box (within two tolerances of a side): the crossing
/// has no end there. `None` where every curve ends so.
fn loose_end(pair: &Pair, m: &Meeting) -> Option<String> {
    let t = pair.tol.linear;
    for (n, curve) in m.curves.iter().enumerate() {
        if curve.closed {
            continue;
        }
        for end in [curve.range.0, curve.range.1] {
            let p = curve.curve.eval(end);
            let at_point = pair.keep_off.iter().any(|c| c.distance(p) <= 2.0 * t);
            let on_box = !pair.within.contains(p, -2.0 * t);
            if !at_point && !on_box {
                let from = pair
                    .keep_off
                    .iter()
                    .map(|c| c.distance(p))
                    .fold(f64::INFINITY, f64::min);
                return Some(format!(
                    "curve {n} ends at {p:?}, {} tolerances from the nearest point where curves of the crossing cross or pass close, inside the box",
                    from / t
                ));
            }
        }
    }
    None
}

/// Viviani's figure and Steinmetz's ellipses opened (the two families
/// above, 1 440 pairs) held to their crossing, and every end of a curve at
/// the point where the march reads a crossing or on the box. The second is
/// what `held` leaves out: it does not look within 50 tolerances of that
/// point (`keep_off_radius`, a twentieth of the radius), and asks a curve
/// to be near the crossing at 97 places of each piece, every 80 tolerances
/// or so along a whole loop. Where a
/// seed inside the zone of such a point was traced again and `united`
/// joined its trace to the branch already traced (26 pairs at `η = 1e-4`
/// before the repair), the joined curve began 80 tolerances out on an arm
/// (where that arm's seed was) instead of at the point, which `held` did
/// not see (probe Check 45673361291, two of them written out).
#[test]
fn opened_figures_are_their_crossing_and_end_only_at_the_point_or_on_the_box() {
    let mut failed = Vec::new();
    let mut counted: BTreeMap<String, usize> = BTreeMap::new();
    for pair in vivianis_opened().iter().chain(steinmetz_opened().iter()) {
        let met = meet_surfaces_numeric(&pair.a, &pair.b, &pair.within, &pair.tol);
        let why = match &met {
            Ok(SurfaceMeeting::Meets(m)) => held(pair, m).err().or_else(|| loose_end(pair, m)),
            other => Some(format!("{other:?}")),
        };
        let family = if pair.what.starts_with("ball") {
            "ball and tube opened"
        } else {
            "two tubes opened"
        };
        *counted.entry(family.to_string()).or_insert(0) += 1;
        if let Some(why) = why {
            failed.push(format!("{}: {why}", pair.what));
        }
    }
    for (family, n) in &counted {
        say(&format!("REVIEW-ENDS {family}: {n} pairs"));
    }
    assert!(
        failed.is_empty(),
        "{} answers are not the crossing ending at its points or the box:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// A small circle where a plane cuts a cap off a ball, the surfaces at
/// 0.02 to 1 radian along it, the circle 10 to 400 tolerances in radius,
/// in cubes of 1.5 to 2 200 radii: no fold is read, and every answer is
/// the crossing.
#[test]
fn a_small_circle_near_a_tangency_reads_no_fold() {
    let pairs = every(|placed, how, pairs| {
        for (alpha, rho) in [(0.02, 4e-4), (0.2, 4e-5), (0.2, 4e-4), (1.0, 1e-5)] {
            for size in sizes(1.5, 22) {
                pairs.push(cap(placed, alpha, rho, size, how));
            }
        }
    });
    run("cap", &pairs, true, true);
}

/// Two cylinders of one radius whose axes cross at 30, 45, 60 and 90
/// degrees, in cubes of 1.5 to 1 090 radii (and of 400) about a point
/// where their ellipses cross: a fold may be read there (a step through
/// that point; at 30 degrees, turned, in the cube of 400 radii at every
/// scale, Check 45287391492), and every answer taken up again is the
/// crossing.
#[test]
fn tubes_crossing_at_an_angle_are_the_crossing_about_their_crossing_points() {
    let pairs = every(|placed, how, pairs| {
        for gamma in [FRAC_PI_6, FRAC_PI_4, FRAC_PI_3, FRAC_PI_2] {
            for size in sizes(1.5, 20).into_iter().chain([400.0]) {
                pairs.push(tubes_at(placed, gamma, size, how));
            }
        }
    });
    run("tubes at an angle", &pairs, false, true);
}
