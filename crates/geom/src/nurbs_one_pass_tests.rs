//! `make_bezier` (all knots in one pass) against
//! `make_bezier_one_at_a_time`, written by the review of 2026-11-17
//! (review-grid-scale) on inputs drawn otherwise than the author's test
//! (`nurbs.rs` › `all_knots_at_once_are_the_knots_one_at_a_time_bit_for_bit`)
//! draws them: degrees up to the largest, long rows, knots
//! with long runs (past the degree, at the ends past the order), zeros of
//! both signs among the knots and the points in sparse and dense runs,
//! subnormal numbers, weights near zero and large, magnitudes near the
//! largest finite number (where a blend of two finite points overflows),
//! points that are not finite, rows of the wrong length. Every
//! difference is counted by its kind and the first of each printed.

use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(0x5851_f42d_4c95_7f2d)
            .wrapping_add(0x1405_7b7e_f767_814f);
        let mut x = self.0;
        x ^= x >> 29;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^ (x >> 32)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn bits_of(rows: &[Vec<H>]) -> Vec<u64> {
    rows.iter()
        .flatten()
        .flat_map(|h| [h.p.x, h.p.y, h.p.z, h.w])
        .map(f64::to_bits)
        .collect()
}

/// A knot vector of `count + degree + 1` knots, of one of several kinds.
fn knots(rng: &mut Rng, degree: usize, count: usize) -> Vec<f64> {
    let n = count + degree + 1;
    let pool = [
        -0.0, 0.0, 0.125, 0.2, 0.25, 0.3, 0.5, 0.6, 0.75, 0.9, 1.0, 1e-300, 7.0, 1e6,
    ];
    let mut k: Vec<f64> = match rng.below(5) {
        // Few distinct values: long runs everywhere.
        0 => {
            let distinct = 2 + rng.below(4);
            (0..n)
                .map(|_| pool[rng.below(distinct.min(pool.len()))])
                .collect()
        }
        // Many distinct values, some repeated.
        1 => (0..n).map(|_| pool[rng.below(pool.len())]).collect(),
        // Uniform and unclamped.
        2 => (0..n).map(|i| i as f64 * 0.5).collect(),
        // Clamped with interior runs of random length (up to degree + 1).
        3 => {
            let mut v = vec![if rng.below(2) == 0 { -0.0 } else { 0.0 }; degree + 1];
            while v.len() < count {
                let value = rng.unit();
                for _ in 0..(1 + rng.below(degree + 1)) {
                    if v.len() < count {
                        v.push(value);
                    }
                }
            }
            v.extend(std::iter::repeat_n(1.0, degree + 1));
            v
        }
        // Random reals.
        _ => (0..n).map(|_| rng.unit() * 4.0 - 2.0).collect(),
    };
    k.sort_by(f64::total_cmp);
    k
}

fn value(rng: &mut Rng, kind: usize) -> f64 {
    match kind {
        0 => rng.unit() * 2.0 - 1.0,
        1 => {
            if rng.below(3) == 0 {
                rng.unit() - 0.5
            } else {
                [0.0, -0.0][rng.below(2)]
            }
        }
        2 => [5e-324, -5e-324, -1e-308, 1e-308, -0.0, 0.0, -2e-320][rng.below(7)],
        3 => [f64::MAX, -f64::MAX, f64::MAX / 3.0, -0.0, 1.0][rng.below(5)],
        4 => (rng.unit() - 0.5) * 1e300,
        _ => [f64::NAN, f64::INFINITY, -f64::INFINITY, 1.0, -0.0][rng.below(5)],
    }
}

#[derive(Default, Debug)]
struct Tally {
    cases: usize,
    one_pass: usize,
    zero_turned: usize,
    differ: usize,
}

/// The two ways on 5 000 rows of the kinds above
/// ([`against_one_at_a_time`]).
#[test]
fn make_bezier_against_one_at_a_time() {
    against_one_at_a_time(5_000);
}

/// [`make_bezier_against_one_at_a_time`] on the review's 200 000 rows (six
/// minutes in a dev build on a loaded laptop).
#[test]
#[ignore = "the review's full run; run with --ignored, in release"]
fn make_bezier_against_one_at_a_time_on_every_row_of_the_review() {
    against_one_at_a_time(200_000);
}

/// The two ways on `cases` rows of the kinds above (the first `cases` of
/// the review's draw): where both give only finite numbers they must be
/// the same bits, knots and points; every case is counted, and the first
/// difference of each kind printed.
fn against_one_at_a_time(cases: usize) {
    let mut rng = Rng(0x0067_7269_6473_6361);
    let mut tally: std::collections::BTreeMap<&str, Tally> = Default::default();
    let mut first_difference: std::collections::BTreeMap<&str, String> = Default::default();
    for case in 0..cases {
        let degree = if case % 20 == 0 {
            1 + rng.below(MAX_DEGREE)
        } else {
            1 + rng.below(7)
        };
        let count = degree
            + 1
            + if case % 50 == 0 {
                rng.below(400)
            } else {
                rng.below(16)
            };
        let k = knots(&mut rng, degree, count);
        // The domain must not be empty (as `check_knots` asks).
        let (a, b) = (k[degree], k[count]);
        if a.partial_cmp(&b) != Some(std::cmp::Ordering::Less) {
            continue;
        }
        let kind = match case % 16 {
            0..=5 => 0,
            6..=10 => 1,
            11 | 12 => 2,
            13 => 3,
            14 => 4,
            _ => 5,
        };
        let class = match kind {
            0 => "plain",
            1 => "zeros of both signs",
            2 => "subnormal",
            3 => "near the largest",
            4 => "1e300",
            _ => "not finite",
        };
        let wrong_length = case % 97 == 0;
        let mut rows: Vec<Vec<H>> = Vec::new();
        for _ in 0..(1 + rng.below(4)) {
            let len = if wrong_length {
                count + 1 - 2 * rng.below(2)
            } else {
                count
            };
            let row = (0..len)
                .map(|_| {
                    let w = match rng.below(5) {
                        0 => 1e-300,
                        1 => 1e10,
                        _ => 0.25 + rng.unit() * 4.0,
                    };
                    let p = Vec3::new(
                        value(&mut rng, kind),
                        value(&mut rng, kind),
                        value(&mut rng, kind),
                    );
                    H { p: p * w, w }
                })
                .collect();
            rows.push(row);
        }
        let class = if wrong_length { "wrong length" } else { class };
        let t = tally.entry(class).or_default();
        t.cases += 1;
        let fits = !wrong_length
            && rows
                .iter()
                .all(|r| r.iter().all(|h| h.p.is_finite() && h.w.is_finite()));
        if fits
            && let Some(plan) = insertions(&k, degree)
            && rows.iter().all(|r| all_inserted(r, &plan).is_some())
        {
            t.one_pass += 1;
        }
        let (mut k1, mut r1) = (k.clone(), rows.clone());
        make_bezier(&mut k1, degree, &mut r1);
        let (mut k2, mut r2) = (k.clone(), rows.clone());
        make_bezier_one_at_a_time(&mut k2, degree, &mut r2);
        let kb = |k: &[f64]| k.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        if kb(&k1) != kb(&k2) || bits_of(&r1) != bits_of(&r2) {
            t.differ += 1;
            first_difference.entry(class).or_insert_with(|| {
                let at = bits_of(&r1)
                    .iter()
                    .zip(bits_of(&r2))
                    .position(|(a, b)| *a != b);
                format!(
                    "case {case}: degree {degree}, knots {k:?}, rows {:?}, first different number at {at:?}: one pass {:?} one at a time {:?}",
                    rows,
                    at.map(|i| f64::from_bits(bits_of(&r1)[i])),
                    at.map(|i| f64::from_bits(bits_of(&r2)[i])),
                )
            });
        }
        let negative_zeros = |rows: &[Vec<H>]| {
            bits_of(rows)
                .iter()
                .filter(|&&b| b == (-0.0_f64).to_bits())
                .count()
        };
        if negative_zeros(&r2) != negative_zeros(&rows) {
            t.zero_turned += 1;
        }
    }
    for (class, t) in &tally {
        println!("{class}: {t:?}");
    }
    for (class, text) in &first_difference {
        let short: String = text.chars().take(3000).collect();
        println!("FIRST DIFFERENCE ({class}): {short}");
    }
    // Where every number is finite throughout, the two ways must agree;
    // the other classes are reported, not held.
    for class in [
        "plain",
        "zeros of both signs",
        "subnormal",
        "1e300",
        "wrong length",
    ] {
        if let Some(t) = tally.get(class) {
            assert_eq!(t.differ, 0, "{class}: {t:?}");
        }
    }
}

/// A row whose blend of two finite points overflows: the one-at-a-time
/// way then copies an infinity (`∞ · 0` is not a number), the one-pass
/// way copies exactly. Printed, not held: what each way gives.
#[test]
fn make_bezier_where_a_blend_overflows() {
    let big = f64::MAX;
    for (label, row) in [
        ("max, max, max, max", vec![big, big, big, big]),
        ("max, -max, max, -max", vec![big, -big, big, -big]),
        (
            "0.9 max, max, 0.9 max, max",
            vec![0.9 * big, big, 0.9 * big, big],
        ),
    ] {
        let degree = 3;
        let mut knots = vec![0.0, 0.0, 0.0, 0.0, 0.3, 1.0 / 3.0, 0.7, 1.0, 1.0, 1.0, 1.0];
        let row: Vec<H> = row
            .iter()
            .chain([big, -big, 0.5 * big].iter())
            .take(7)
            .map(|&x| H {
                p: Vec3::new(x, -x, 0.0),
                w: 1.0,
            })
            .collect();
        let (mut k1, mut r1) = (knots.clone(), vec![row.clone()]);
        make_bezier(&mut k1, degree, &mut r1);
        let mut r2 = vec![row];
        make_bezier_one_at_a_time(&mut knots, degree, &mut r2);
        let _ = k1;
        let finite = |r: &[Vec<H>]| r.iter().flatten().filter(|h| h.p.is_finite()).count();
        println!(
            "OVERFLOW {label}: same bits {}, finite points one pass {} / one at a time {} of {}",
            bits_of(&r1) == bits_of(&r2),
            finite(&r1),
            finite(&r2),
            r1.iter().flatten().count()
        );
    }
}

/// A blend that overflows: Boehm's factor `α` below 2⁻⁵³ makes
/// `1 − α` round to one, so `P·(1 − α) + Q·α` of two points at the
/// largest finite number is past it: an infinity. The one-at-a-time way
/// then copies its neighbours as `∞ · 0 + x`, which is not a number; the
/// one-pass way copies them exactly. Printed, then held.
#[test]
fn make_bezier_where_a_tiny_factor_overflows() {
    let big = f64::MAX;
    let degree = 3;
    let tiny = 1.0 / (1_u64 << 54) as f64;
    let knots = vec![0.0, 0.0, 0.0, 0.0, tiny, 1.0, 1.0, 1.0, 1.0];
    let row: Vec<H> = (0..5)
        .map(|i| H {
            p: Vec3::new(big, if i % 2 == 0 { 1.0 } else { -1.0 }, 0.0),
            w: 1.0,
        })
        .collect();
    let (mut k1, mut r1) = (knots.clone(), vec![row.clone()]);
    make_bezier(&mut k1, degree, &mut r1);
    let (mut k2, mut r2) = (knots.clone(), vec![row]);
    make_bezier_one_at_a_time(&mut k2, degree, &mut r2);
    let show = |r: &[Vec<H>]| {
        r.iter()
            .flatten()
            .map(|h| format!("({:e}, {:e})", h.p.x, h.p.y))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!("TINY one pass:     {}", show(&r1));
    println!("TINY one at a time: {}", show(&r2));
    println!("TINY same bits: {}", bits_of(&r1) == bits_of(&r2));
    // The surface made of such a row: what NurbsSurface gives.
    let ctrl: Vec<Vec3> = (0..10)
        .map(|i| Vec3::new(big, (i % 2) as f64, (i / 5) as f64))
        .collect();
    let s = NurbsSurface::new(
        3,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        5,
        2,
        ctrl,
        vec![1.0; 10],
    )
    .unwrap();
    let spans = s.bezier_spans().unwrap();
    let nan = spans
        .iter()
        .flat_map(|sp| sp.patch.control_points())
        .filter(|p| p.x.is_nan())
        .count();
    println!(
        "TINY surface: {} spans, {nan} control points not a number",
        spans.len()
    );
}

/// What the one-pass plan holds for the largest curve the caps allow:
/// degree 32, 65 536 points, every interior knot single (each inserted
/// 31 times), against the row the spans make: no more than the row (the
/// plan kept Boehm's factors until 2026-11-17, 585 MB against the row's
/// 67 MB; now two numbers an insertion and the knots).
#[test]
fn plan_of_the_largest_curve() {
    let (degree, count) = (MAX_DEGREE, MAX_CONTROL_POINTS);
    let knots = largest_curves_knots();
    let t = std::time::Instant::now();
    let plan = insertions(&knots, degree).unwrap();
    let bytes = plan.steps.len() * std::mem::size_of::<Insertion>()
        + (plan.given.len() + plan.after.len()) * std::mem::size_of::<f64>();
    let row = (plan.after.len() - degree - 1) * std::mem::size_of::<H>();
    println!(
        "PLAN degree {degree}, {count} points: {} insertions, {:.0} MB; the row of spans {:.0} MB; made in {:?}",
        plan.steps.len(),
        bytes as f64 / 1e6,
        row as f64 / 1e6,
        t.elapsed()
    );
    assert!(bytes <= row, "the plan {bytes} bytes, the row {row}");
}

/// The knots of the largest curve the caps allow: degree 32, 65 536
/// points, every interior knot single.
fn largest_curves_knots() -> Vec<f64> {
    let (degree, count) = (MAX_DEGREE, MAX_CONTROL_POINTS);
    let inner = count - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..=inner).map(|i| i as f64 / (inner + 1) as f64));
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

/// The process's peak resident memory so far, in MB (Linux; `None`
/// elsewhere).
fn peak_mb() -> Option<f64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<f64>()
        .ok()
        .map(|kib| kib * 1024.0 / 1e6)
}

/// The spans of the largest curve the caps allow, one pass: the process's
/// peak memory before and after, printed. Run alone (`--ignored
/// --nocapture` with this name): the peak is the process's.
#[test]
#[ignore = "prints the peak memory of the largest curve's spans; run alone"]
fn peak_memory_of_the_largest_curves_spans() {
    let (degree, count) = (MAX_DEGREE, MAX_CONTROL_POINTS);
    let mut knots = largest_curves_knots();
    let mut rng = Rng(7);
    let mut rows = vec![
        (0..count)
            .map(|_| H {
                p: Vec3::new(rng.unit(), rng.unit(), rng.unit()),
                w: 1.0,
            })
            .collect::<Vec<H>>(),
    ];
    let before = peak_mb();
    let t = std::time::Instant::now();
    make_bezier(&mut knots, degree, &mut rows);
    let row = rows.iter().map(Vec::len).sum::<usize>() * std::mem::size_of::<H>();
    println!(
        "PEAK degree {degree}, {count} points: {before:?} MB before, {:?} MB after; the row of spans {:.0} MB; {:?}",
        peak_mb(),
        row as f64 / 1e6,
        t.elapsed()
    );
}
